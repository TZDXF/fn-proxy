use crate::{
    error::{error, Result},
    text::Text,
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

// Diagnostics contain only fixed field names, array indices and JSON types, never values.
fn json_kind(value: Option<&Value>) -> &'static str {
    match value {
        None => "missing",
        Some(Value::Null) => "null",
        Some(Value::Bool(_)) => "boolean",
        Some(Value::Number(_)) => "number",
        Some(Value::String(_)) => "string",
        Some(Value::Array(_)) => "array",
        Some(Value::Object(_)) => "object",
    }
}
fn require_kind(value: Option<&Value>, path: String, expected: &str, optional: bool) -> Result<()> {
    let actual = json_kind(value);
    if actual == expected || (optional && matches!(actual, "missing" | "null")) {
        return Ok(());
    }
    Err(crate::error::error_with(
        "containers.schemaDetail",
        [
            ("path", path),
            ("expected", expected.to_owned()),
            ("actual", actual.to_owned()),
        ],
    ))
}
fn normalize_fields(value: &mut Value, path: &str, fields: &[(&str, &str)]) -> Result<()> {
    let Some(object) = value.as_object_mut() else {
        return Ok(());
    };
    for &(canonical, alternate) in fields {
        if object.contains_key(canonical) && object.contains_key(alternate) {
            return Err(crate::error::error_with(
                "containers.schemaDetail",
                [
                    ("path", format!("{path}.{canonical}")),
                    ("expected", "single field".to_owned()),
                    ("actual", "duplicate aliases".to_owned()),
                ],
            ));
        }
        if let Some(field) = object.remove(alternate) {
            object.insert(canonical.to_owned(), field);
        }
    }
    Ok(())
}
pub fn parse_container_packet(text: &str) -> Result<ContainerPacket> {
    let mut value: Value =
        serde_json::from_str(text).map_err(|_| error("containers.invalidJson"))?;
    if let Some(items) = value.get_mut("rsp").and_then(Value::as_array_mut) {
        for (i, item) in items.iter_mut().enumerate() {
            let path = format!("rsp[{i}]");
            normalize_fields(
                item,
                &path,
                &[
                    ("id", "Id"),
                    ("names", "Names"),
                    ("state", "State"),
                    ("ports", "Ports"),
                ],
            )?;
            if item.get("id").is_none() {
                let shape = [
                    "ID",
                    "containerId",
                    "containerID",
                    "data",
                    "list",
                    "containers",
                    "name",
                    "status",
                ]
                .iter()
                .map(|field| format!("{field}:{}", json_kind(item.get(*field))))
                .collect::<Vec<_>>()
                .join(", ");
                return Err(crate::error::error_with(
                    "containers.schemaCandidates",
                    [("path", format!("{path}.id")), ("shape", shape)],
                ));
            }
            if let Some(ports) = item.get_mut("ports").and_then(Value::as_array_mut) {
                for (j, port) in ports.iter_mut().enumerate() {
                    normalize_fields(
                        port,
                        &format!("{path}.ports[{j}]"),
                        &[
                            ("publicPort", "PublicPort"),
                            ("privatePort", "PrivatePort"),
                            ("type", "Type"),
                            ("ip", "IP"),
                        ],
                    )?;
                }
            }
        }
    }
    require_kind(Some(&value), "$".to_owned(), "object", false)?;
    require_kind(value.get("result"), "result".to_owned(), "string", true)?;
    if let Some(errno) = value.get("errno").filter(|v| !v.is_null()) {
        if errno.as_i64().is_none() {
            return Err(crate::error::error_with(
                "containers.schemaDetail",
                [
                    ("path", "errno".to_owned()),
                    ("expected", "integer".to_owned()),
                    ("actual", json_kind(Some(errno)).to_owned()),
                ],
            ));
        }
    }
    require_kind(value.get("rsp"), "rsp".to_owned(), "array", true)?;
    if let Some(items) = value.get("rsp").and_then(Value::as_array) {
        for (i, item) in items.iter().enumerate() {
            let base = format!("rsp[{i}]");
            require_kind(Some(item), base.clone(), "object", false)?;
            require_kind(item.get("id"), format!("{base}.id"), "string", false)?;
            require_kind(
                item.get("state"),
                format!("{base}.state"),
                "string",
                item.get("state").is_none(),
            )?;
            for field in ["names", "ports"] {
                require_kind(item.get(field), format!("{base}.{field}"), "array", true)?;
            }
            if let Some(names) = item.get("names").and_then(Value::as_array) {
                for (j, name) in names.iter().enumerate() {
                    require_kind(Some(name), format!("{base}.names[{j}]"), "string", false)?;
                }
            }
            if let Some(ports) = item.get("ports").and_then(Value::as_array) {
                for (j, port) in ports.iter().enumerate() {
                    let path = format!("{base}.ports[{j}]");
                    require_kind(Some(port), path.clone(), "object", false)?;
                    require_kind(
                        port.get("type"),
                        format!("{path}.type"),
                        "string",
                        port.get("type").is_none(),
                    )?;
                    require_kind(port.get("ip"), format!("{path}.ip"), "string", true)?;
                }
            }
        }
    }
    serde_json::from_value(value).map_err(|_| error("containers.schemaChanged"))
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
            return Err(error("containers.alreadyFinished"));
        }
        if packet.errno.is_some_and(|code| code != 0)
            || matches!(packet.result.as_deref(), Some("fail" | "cancel"))
        {
            return Err(error("containers.denied"));
        }
        let finished = matches!(packet.result.as_deref(), Some("suc" | "succ"));
        if !finished && !matches!(packet.result.as_deref(), Some("doing") | None) {
            return Err(error("containers.unknownState"));
        }
        for mut item in packet.rsp.unwrap_or_default() {
            self.records += 1;
            self.bindings += item.ports.len();
            if self.records > self.limits.records || self.bindings > self.limits.bindings {
                return Err(error("containers.overLimit"));
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
                return Err(error("containers.unsupportedMetadata"));
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
            return Err(error("containers.missingSuccess"));
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
        Err(e) => {
            report.sources.push(InventorySource {
                id: "containers".to_owned(),
                name: Text::new("containers.sourceName"),
                status: "unavailable".to_owned(),
                count: None,
                message: match &e {
                    crate::error::AppError::Io(_) => Text::new("containers.disconnected"),
                    _ => e.text(),
                },
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
                ("not-published", Text::new("containers.noHostPort"))
            } else if protocol != "tcp" {
                (
                    "unsupported-protocol",
                    Text::new("containers.unsupportedProtocol"),
                )
            } else if !registry_available {
                (
                    "registry-unavailable",
                    Text::new("containers.registryUnavailable"),
                )
            } else if ambiguous_prefix || candidates.len() > 1 {
                ("ambiguous", Text::new("containers.ambiguous"))
            } else if candidates.is_empty() {
                ("no-domain", Text::new("containers.noDomain"))
            } else {
                ("mapped", Text::new("containers.mapped"))
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
                reason,
            });
        }
    }
    let published_ports = rows.iter().filter(|row| row.nas_port.is_some()).count();
    report.sources.push(InventorySource {
        id: "containers".to_owned(),
        name: Text::new("containers.sourceName"),
        status: "ok".to_owned(),
        count: Some(items.len()),
        message: Text::with(
            "containers.summary",
            [
                ("containers", items.len().to_string()),
                ("publishedPorts", published_ports.to_string()),
            ],
        ),
    });
    report.docker = Some(DockerPortInventory {
        containers: items.len(),
        published_ports,
        registry_available,
        mapped_ports: rows.iter().filter(|row| row.status == "mapped").count(),
        unmapped_ports: rows.iter().filter(|row| row.status == "no-domain").count(),
        unconfirmed_ports: rows
            .iter()
            .filter(|row| matches!(row.status.as_str(), "registry-unavailable" | "ambiguous"))
            .count(),
        rows,
        scope: Text::new("containers.scope"),
    });
}

#[cfg(test)]
#[path = "docker_tests.rs"]
mod tests;
