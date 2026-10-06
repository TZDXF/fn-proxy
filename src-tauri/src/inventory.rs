use crate::{
    error::{error, Result},
    types::{
        normalize_fnid, validate_upstream, DiscoveredService, InventoryEntry, ServiceInventory,
    },
};
use serde_json::Value;
use std::collections::HashSet;

/// Preserve every visible entry, including ones that cannot become a remote-port route.
/// This is an account-scoped registration inventory, never a listener/port scan.
pub fn parse_inventory(value: &Value, fn_id: &str) -> Result<ServiceInventory> {
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
        let name = entry["title"].as_str().unwrap_or("NAS 入口").to_owned();
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
                    source: "NAS 当前账号入口列表 uri.port ↔ uri.fnDomain".to_owned(),
                });
            }
        }
        entries.push(InventoryEntry {
            id: format!("{}:{index}", key.unwrap_or("entry")),
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
        total_entries: entries.len(), mapped_entries,
        unmapped_entries: entries.len() - mapped_entries,
        services, entries,
        scope: "当前登录账号在 getEntryList 中可见的已注册入口；不等于 NAS 所有监听端口，也不保证入口当前可达".to_owned(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
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
