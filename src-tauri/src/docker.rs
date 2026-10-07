use crate::{
    error::{error, Result},
    types::{DockerPortInventory, DockerPortRow, InventorySource, ServiceInventory},
};
use serde::{Deserialize, Deserializer};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use std::time::Duration;

#[derive(Clone, Copy)]
pub struct StreamLimits {
    pub total: Duration,
    pub idle: Duration,
    pub frame_bytes: usize,
    pub packets: usize,
    pub records: usize,
    pub bindings: usize,
}
impl Default for StreamLimits {
    fn default() -> Self {
        Self {
            total: Duration::from_secs(45),
            idle: Duration::from_secs(10),
            frame_bytes: 2 * 1024 * 1024,
            packets: 1024,
            records: 10_000,
            bindings: 20_000,
        }
    }
}
fn optional_port<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> std::result::Result<Option<u16>, D::Error> {
    let value = Value::deserialize(deserializer)?;
    Ok(value
        .as_u64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
        .and_then(|p| u16::try_from(p).ok())
        .filter(|p| *p != 0))
}
fn null_vec<'de, D, T>(deserializer: D) -> std::result::Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Ok(Option::<Vec<T>>::deserialize(deserializer)?.unwrap_or_default())
}
// Whitelisted metadata only: unknown container fields (including env/config) are ignored.
#[derive(Clone, Deserialize, Debug, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct PublishedPort {
    #[serde(default, deserialize_with = "optional_port")]
    pub public_port: Option<u16>,
    #[serde(default, deserialize_with = "optional_port")]
    pub private_port: Option<u16>,
    #[serde(default, rename = "type")]
    pub protocol: String,
    #[serde(default)]
    pub ip: Option<String>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContainerMetadata {
    pub id: String,
    #[serde(default, deserialize_with = "null_vec")]
    pub names: Vec<String>,
    #[serde(default)]
    pub state: String,
    #[serde(default, deserialize_with = "null_vec")]
    pub ports: Vec<PublishedPort>,
}
#[derive(Deserialize)]
pub struct StreamMatch {
    #[serde(default)]
    pub reqid: Option<Value>,
}
#[derive(Deserialize)]
pub struct ContainerPacket {
    #[serde(default)]
    pub result: Option<String>,
    #[serde(default)]
    pub errno: Option<i64>,
    #[serde(default)]
    pub rsp: Option<Vec<ContainerMetadata>>,
}

pub struct ContainerCollector {
    items: Vec<ContainerMetadata>,
    positions: HashMap<String, usize>,
    records: usize,
    bindings: usize,
    finished: bool,
    limits: StreamLimits,
}
impl ContainerCollector {
    pub fn new(limits: StreamLimits) -> Self {
        Self {
            items: Vec::new(),
            positions: HashMap::new(),
            records: 0,
            bindings: 0,
            finished: false,
            limits,
        }
    }
    /// Only a successful terminal packet yields an inventory; partial results stay private.
    pub fn push(&mut self, packet: ContainerPacket) -> Result<bool> {
        if self.finished {
            return Err(error("容器清单已经完成，拒绝混入后续响应"));
        }
        if packet.errno.is_some_and(|code| code != 0)
            || matches!(packet.result.as_deref(), Some("fail" | "cancel"))
        {
            return Err(error("NAS 拒绝或取消容器端口盘点，请检查 Docker 权限"));
        }
        let finished = matches!(packet.result.as_deref(), Some("suc" | "succ"));
        if !finished && !matches!(packet.result.as_deref(), Some("doing") | None) {
            return Err(error("容器列表返回未知状态，未将分段数据当作完整清单"));
        }
        for mut item in packet.rsp.unwrap_or_default() {
            self.records += 1;
            self.bindings += item.ports.len();
            if self.records > self.limits.records || self.bindings > self.limits.bindings {
                return Err(error("容器清单超过本工具安全上限；未截断后冒充完整清单"));
            }
            if item.id.is_empty()
                || item.id.len() > 256
                || item.names.len() > 32
                || item.names.iter().any(|name| name.len() > 512)
                || item.state.len() > 64
                || item.ports.iter().any(|port| {
                    port.protocol.len() > 16 || port.ip.as_ref().is_some_and(|ip| ip.len() > 128)
                })
            {
                return Err(error("容器清单包含不受支持的标识或元数据，未生成猜测映射"));
            }
            let mut unique_ports = HashSet::new();
            for port in &mut item.ports {
                port.protocol = port.protocol.to_ascii_lowercase();
            }
            item.ports.retain(|port| unique_ports.insert(port.clone()));
            if let Some(&index) = self.positions.get(&item.id) {
                let previous = &mut self.items[index];
                if !item.names.is_empty() {
                    previous.names = item.names;
                }
                if !item.state.is_empty() {
                    previous.state = item.state;
                }
                for port in item.ports {
                    if !previous.ports.contains(&port) {
                        previous.ports.push(port);
                    }
                }
            } else {
                self.positions.insert(item.id.clone(), self.items.len());
                self.items.push(item);
            }
        }
        self.finished = finished;
        Ok(finished)
    }
    pub fn finish(self) -> Result<Vec<ContainerMetadata>> {
        if !self.finished {
            return Err(error("容器清单缺少成功终态，不能返回分段残留"));
        }
        Ok(self.items)
    }
}

pub fn attach_container_ports(
    report: &mut ServiceInventory,
    containers: Result<Vec<ContainerMetadata>>,
) {
    let items = match containers {
        Ok(items) => items,
        Err(_) => {
            report.sources.push(InventorySource {
                id: "containers".to_owned(),
                name: "Docker 容器端口盘点".to_owned(),
                status: "unavailable".to_owned(),
                count: None,
                message: "盘点失败或未收到成功终态；不展示分段残留，也不把未知计为零端口"
                    .to_owned(),
            });
            return;
        }
    };
    let registry_available = report
        .sources
        .iter()
        .any(|source| source.id == "docker" && source.status == "ok");
    let mut rows = Vec::new();
    for container in &items {
        let name = container
            .names
            .first()
            .map(|name| name.trim_start_matches('/').to_owned())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| container.id.chars().take(12).collect());
        for (index, port) in container.ports.iter().enumerate() {
            let protocol = port.protocol.to_ascii_lowercase();
            let mut candidates = Vec::new();
            let mut ambiguous_prefix = false;
            if protocol == "tcp" && port.public_port.is_some() && registry_available {
                let mut seen = HashSet::new();
                for entry in &report.entries {
                    let matches_container = entry.app_id.as_ref().is_some_and(|prefix| {
                        !prefix.is_empty() && container.id.starts_with(prefix)
                    });
                    if matches_container
                        && entry.nas_port == port.public_port
                        && entry.status == "mapped"
                    {
                        if let Some(prefix) = &entry.app_id {
                            if items
                                .iter()
                                .filter(|item| item.id.starts_with(prefix))
                                .count()
                                != 1
                            {
                                ambiguous_prefix = true;
                                continue;
                            }
                        }
                        if let (Some(upstream), Some(domain)) = (&entry.upstream, &entry.fn_domain)
                        {
                            if seen.insert(upstream.clone()) {
                                candidates.push((upstream.clone(), domain.clone()));
                            }
                        }
                    }
                }
            }
            let (status, reason) = if port.public_port.is_none() {
                (
                    "not-published",
                    "未提供有效宿主机端口；容器内暴露端口不能直接作为远程代理目标",
                )
            } else if protocol != "tcp" {
                (
                    "unsupported-protocol",
                    "仅展示此发布记录；本工具不转发 UDP 或未知协议",
                )
            } else if !registry_available {
                (
                    "registry-unavailable",
                    "Docker 远程映射来源未读取，不能判断是否已经分配域名",
                )
            } else if ambiguous_prefix || candidates.len() > 1 {
                (
                    "ambiguous",
                    "容器前缀存在歧义或匹配多个远程域名，不自动任选",
                )
            } else if candidates.is_empty() {
                (
                    "no-domain",
                    "没有匹配到此容器及宿主机端口的有效远程域名；不按全局端口号猜测",
                )
            } else {
                (
                    "mapped",
                    "容器 ID 前缀与宿主机端口均匹配；已注册不代表服务或 HTTP API 实际可达",
                )
            };
            let pair = if status == "mapped" {
                candidates.pop()
            } else {
                None
            };
            rows.push(DockerPortRow {
                id: format!("{}:{index}", container.id),
                container_id: container.id.clone(),
                container_name: name.clone(),
                state: container.state.clone(),
                protocol,
                host_ip: port.ip.clone(),
                nas_port: port.public_port,
                container_port: port.private_port,
                upstream: pair.as_ref().map(|pair| pair.0.clone()),
                fn_domain: pair.map(|pair| pair.1),
                status: status.to_owned(),
                reason: reason.to_owned(),
            });
        }
    }
    let published_ports = rows.iter().filter(|row| row.nas_port.is_some()).count();
    report.sources.push(InventorySource {
        id: "containers".to_owned(),
        name: "Docker 容器端口盘点".to_owned(),
        status: "ok".to_owned(),
        count: Some(items.len()),
        message: format!(
            "成功终态后汇总 {} 个容器、{published_ports} 条发布记录；仅保留名称、状态与端口元数据",
            items.len()
        ),
    });
    report.docker = Some(DockerPortInventory {
        containers: items.len(), published_ports, registry_available,
        mapped_ports: rows.iter().filter(|row| row.status == "mapped").count(),
        unmapped_ports: rows.iter().filter(|row| row.status == "no-domain").count(),
        unconfirmed_ports: rows.iter().filter(|row| matches!(row.status.as_str(), "registry-unavailable" | "ambiguous")).count(),
        rows,
        scope: "当前账号 containerList 返回的容器端口元数据；包含 TCP/UDP 发布记录及仅暴露端口，不枚举 host 网络模式或 NAS 非 Docker 监听端口".to_owned(),
    });
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;
