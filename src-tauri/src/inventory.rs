use crate::{
    error::{error, Result},
    types::{
        normalize_fnid, validate_upstream, DiscoveredService, InventoryEntry, InventorySource,
        ServiceInventory,
    },
};
use serde_json::Value;
use std::collections::HashSet;

/// Preserve every visible entry, including ones that cannot become a remote-port route.
/// This is an account-scoped registration inventory, never a listener/port scan.
pub fn parse_inventory(value: &Value, fn_id: &str) -> Result<ServiceInventory> {
    parse_source(value, fn_id, "desktop", "NAS 桌面入口")
}
pub fn parse_docker_inventory(value: &Value, fn_id: &str) -> Result<ServiceInventory> {
    parse_source(value, fn_id, "docker", "Docker 快捷访问")
}
fn parse_source(
    value: &Value,
    fn_id: &str,
    source_id: &str,
    source_name: &str,
) -> Result<ServiceInventory> {
    let fn_id = normalize_fnid(fn_id)?;
    let list = value["data"]["list"]
        .as_array()
        .ok_or_else(|| error("NAS 入口响应缺少 data.list，无法确认清单是否完整"))?;
    let mut entries = Vec::new();
    let mut services = Vec::new();
    let mut seen = HashSet::new();
    for (index, entry) in list.iter().enumerate() {
        let uri = &entry["uri"];
        let port = uri["port"]
            .as_u64()
            .or_else(|| uri["port"].as_str().and_then(|s| s.parse().ok()))
            .and_then(|p| u16::try_from(p).ok())
            .filter(|p| *p != 0);
        let app_id = if source_id == "docker" {
            entry["appID"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        } else {
            None
        };
        let fallback_name = if source_id == "docker" {
            format!(
                "Docker 服务 :{}",
                port.map(|p| p.to_string())
                    .unwrap_or_else(|| "未知端口".to_owned())
            )
        } else {
            "NAS 入口".to_owned()
        };
        let name = entry["title"]
            .as_str()
            .filter(|s| !s.is_empty())
            .unwrap_or(&fallback_name)
            .to_owned();
        let key = entry["entryKey"].as_str().filter(|s| !s.is_empty());
        let domain = uri["fnDomain"].as_str().filter(|s| !s.is_empty());
        let candidate = domain.map(|d| format!("https://{d}.{fn_id}.fnos.net/"));
        let valid = candidate
            .as_deref()
            .and_then(|u| validate_upstream(u, &fn_id).ok());
        let upstream = valid.as_ref().map(|url| url.to_string());
        // Do not display query/fragment credentials or arbitrary host addresses from entry metadata.
        let path = uri["path"]
            .as_str()
            .filter(|p| p.starts_with('/') && !p.starts_with("//"))
            .and_then(|p| {
                url::Url::parse("https://placeholder.invalid/")
                    .ok()?
                    .join(p)
                    .ok()
            })
            .map(|url| url.path().to_owned());
        let (status, reason) = if port.is_none() {
            (
                "no-port",
                "没有有效独立端口；可能是 NAS 内置入口或相对路径，不能据此建立端口代理",
            )
        } else if domain.is_none() {
            (
                "no-domain",
                "有端口但没有 fnDomain，尚未得到 FN Connect 服务子域名",
            )
        } else if valid.is_none() {
            (
                "invalid-domain",
                "远程子域名不符合当前 FN ID 的安全规则，已拒绝",
            )
        } else {
            (
                "mapped",
                "服务端提供了端口与子域名配对；实际可达性仍需单独测试",
            )
        };
        if let (Some(port), Some(domain), Some(upstream)) = (port, domain, upstream.as_ref()) {
            if seen.insert((port, upstream.clone())) {
                services.push(DiscoveredService {
                    id: key.unwrap_or(domain).to_owned(),
                    name: name.clone(),
                    nas_port: port,
                    upstream: upstream.clone(),
                    fn_domain: domain.to_owned(),
                    source: format!("{source_name}：uri.port ↔ uri.fnDomain"),
                });
            }
        }
        entries.push(InventoryEntry {
            id: format!("{source_id}:{}:{index}", key.unwrap_or("entry")),
            source: source_name.to_owned(),
            app_id,
            name,
            nas_port: port,
            fn_domain: valid.as_ref().and(domain).map(str::to_owned),
            upstream: if port.is_some() { upstream } else { None },
            path,
            status: status.to_owned(),
            reason: reason.to_owned(),
        });
    }
    let mapped_entries = entries.iter().filter(|e| e.status == "mapped").count();
    Ok(ServiceInventory {
        docker: None,
        sources: vec![InventorySource {
            id: source_id.to_owned(),
            name: source_name.to_owned(),
            status: "ok".to_owned(),
            count: Some(entries.len()),
            message: "当前账号可见的注册映射已读取；不等于全部监听端口".to_owned(),
        }],
        total_entries: entries.len(),
        mapped_entries,
        unmapped_entries: entries.len() - mapped_entries,
        services,
        entries,
        scope: "当前账号可见的已注册入口；不等于 NAS 所有监听端口，也不保证入口当前可达".to_owned(),
    })
}

/// A permission/version failure in one source must not hide entries from another source.
pub fn merge_inventories(
    desktop: Result<ServiceInventory>,
    docker: Result<ServiceInventory>,
) -> Result<ServiceInventory> {
    let mut sources = Vec::new();
    let mut entries = Vec::new();
    let mut services = Vec::new();
    let mut seen = HashSet::new();
    let mut successful_sources = 0;
    for (id, name, result) in [
        ("desktop", "NAS 桌面入口", desktop),
        ("docker", "Docker 快捷访问", docker),
    ] {
        match result {
            Ok(report) => {
                successful_sources += 1;
                sources.extend(report.sources);
                entries.extend(report.entries);
                for service in report.services {
                    if seen.insert((service.nas_port, service.upstream.clone())) {
                        services.push(service);
                    }
                }
            }
            Err(_) => sources.push(InventorySource {
                id: id.to_owned(),
                name: name.to_owned(),
                status: "unavailable".to_owned(),
                count: None,
                message:
                    "读取失败：可能无权限、版本不支持、响应结构变化或连接异常；该来源不计为空列表"
                        .to_owned(),
            }),
        }
    }
    if successful_sources == 0 {
        return Err(error(
            "桌面入口与 Docker 映射均无法读取，请检查 NAS 登录、权限及版本",
        ));
    }
    let mapped_entries = entries.iter().filter(|e| e.status == "mapped").count();
    Ok(ServiceInventory {
        docker: None,
        sources, total_entries: entries.len(), mapped_entries, unmapped_entries: entries.len() - mapped_entries,
        entries, services,
        scope: "合并当前账号可见的 NAS 桌面与 Docker 快捷访问注册映射；部分来源失败会单独标注，不代表 NAS 全部监听端口".to_owned(),
    })
}

pub fn unavailable_registries() -> ServiceInventory {
    ServiceInventory {
        docker: None,
        sources: [("desktop", "NAS 桌面入口"), ("docker", "Docker 快捷访问")]
            .into_iter()
            .map(|(id, name)| InventorySource {
                id: id.to_owned(),
                name: name.to_owned(),
                status: "unavailable".to_owned(),
                count: None,
                message: "远程注册来源未读取；容器发布端口不能据此推断远程域名".to_owned(),
            })
            .collect(),
        total_entries: 0,
        mapped_entries: 0,
        unmapped_entries: 0,
        services: Vec::new(),
        entries: Vec::new(),
        scope: "仅取得容器发布端口元数据，远程注册来源未读取；不宣称端口已有 FN Connect 域名"
            .to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn docker_registry_adds_missing_services_and_preserves_container_association() {
        let desktop = parse_inventory(
            &json!({"data":{"list":[
                {"entryKey":"app:a","uri":{"port":8084,"fnDomain":"shared-0"}}
            ]}}),
            "my-nas",
        );
        let docker = parse_docker_inventory(
            &json!({"data":{"list":[
                {"appID":"fixture-container-prefix","uri":{"port":"8084","fnDomain":"shared-0"}},
                {"appID":"fixture-container-prefix","uri":{"port":"3000","fnDomain":"docker-0"}},
                {"appID":"fixture-other","uri":{"port":"5000"}}
            ]}}),
            "my-nas",
        );
        let report = merge_inventories(desktop, docker).unwrap();
        assert_eq!(report.total_entries, 4);
        assert_eq!(report.mapped_entries, 3);
        assert_eq!(report.unmapped_entries, 1);
        assert_eq!(report.services.len(), 2);
        assert_eq!(report.services[1].nas_port, 3000);
        assert_eq!(
            report.entries[1].app_id.as_deref(),
            Some("fixture-container-prefix")
        );
        assert_eq!(report.entries[1].source, "Docker 快捷访问");
        assert!(report.entries[0].id.starts_with("desktop:"));
        assert!(report.entries[1].id.starts_with("docker:"));
        assert_eq!(report.sources[1].count, Some(3));
    }
    #[test]
    fn denied_or_malformed_source_is_not_reported_as_empty() {
        let empty_desktop = || parse_inventory(&json!({"data":{"list":[]}}), "my-nas");
        let empty_docker = || parse_docker_inventory(&json!({"data":{"list":[]}}), "my-nas");
        let malformed = parse_docker_inventory(&json!({"data":{}}), "my-nas");
        let report = merge_inventories(empty_desktop(), malformed).unwrap();
        assert_eq!(report.sources[1].status, "unavailable");
        assert_eq!(report.sources[1].count, None);
        let report = merge_inventories(Err(error("fixture denied")), empty_docker()).unwrap();
        assert_eq!(report.sources[0].status, "unavailable");
        assert_eq!(report.sources[1].status, "ok");
        assert!(
            merge_inventories(Err(error("fixture denied")), Err(error("fixture denied"))).is_err()
        );
    }
    #[test]
    fn complete_inventory_preserves_unmapped_builtin_and_duplicate_entries() {
        let report = parse_inventory(&json!({"data":{"list":[
            {"entryKey":"app:a","title":"App A","uri":{"port":"8084","fnDomain":"fixture-0","path":"/ui?token=never-display#secret"}},
            {"entryKey":"app:b","title":"App B","uri":{"port":8084,"fnDomain":"fixture-0"}},
            {"entryKey":"app:local","uri":{"port":"9090"}},
            {"entryKey":"builtin","uri":{"path":"/app/builtin"}},
            {"entryKey":"unsafe","uri":{"port":9000,"fnDomain":"evil.test/"}}
        ]}}), "my-nas").unwrap();
        assert_eq!(report.total_entries, 5);
        assert_eq!(report.mapped_entries, 2);
        assert_eq!(report.unmapped_entries, 3);
        assert_eq!(report.services.len(), 1);
        assert_eq!(report.entries[0].path.as_deref(), Some("/ui"));
        assert_eq!(report.entries[2].status, "no-domain");
        assert_eq!(report.entries[2].nas_port, Some(9090));
        assert_eq!(report.entries[3].status, "no-port");
        assert_eq!(report.entries[4].status, "invalid-domain");
        assert!(report.entries[4].upstream.is_none());
        assert!(report.entries[4].fn_domain.is_none());
    }
    #[test]
    fn malformed_or_truncated_results_do_not_silently_become_empty_inventory() {
        assert!(parse_inventory(&json!({"data":{}}), "my-nas").is_err());
        assert!(parse_inventory(&json!({"data":{"list":{}}}), "my-nas").is_err());
        assert!(parse_inventory(&json!({"data":{"list":[]}}), "invalid/id").is_err());
        assert_eq!(
            parse_inventory(&json!({"data":{"list":[]}}), "my-nas")
                .unwrap()
                .total_entries,
            0
        );
    }
    #[test]
    fn invalid_ports_and_external_paths_never_create_proxy_routes() {
        let report = parse_inventory(
            &json!({"data":{"list":[
                {"uri":{"port":65536,"fnDomain":"fixture-0","path":"//outside.test/"}},
                {"uri":{"port":0,"fnDomain":"fixture-0"}},
                {"uri":{"port":"-1","fnDomain":"fixture-0"}},
                {"uri":{"port":true,"fnDomain":"fixture-0"}}
            ]}}),
            "my-nas",
        )
        .unwrap();
        assert!(report.services.is_empty());
        assert_eq!(report.unmapped_entries, 4);
        assert!(report.entries[0].path.is_none());
    }
}
