use crate::error::{error, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    #[serde(default = "default_connection_id")]
    pub id: String,
    pub fn_id: String,
    pub username: String,
    #[serde(default)]
    pub remember: bool,
    #[serde(default)]
    pub auto_connect: bool,
    #[serde(default)]
    pub services: Vec<ServiceRoute>,
}
fn default_connection_id() -> String {
    "default".to_owned()
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceProfiles {
    #[serde(default)]
    pub allow_lan_access: bool,
    pub profiles: Vec<Profile>,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ServiceRoute {
    pub id: String,
    pub name: String,
    pub nas_port: u16,
    pub local_port: u16,
    pub upstream: String,
    #[serde(default = "yes")]
    pub enabled: bool,
}
fn yes() -> bool {
    true
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredService {
    pub id: String,
    pub name: String,
    pub nas_port: u16,
    pub upstream: String,
    pub fn_domain: String,
    pub source: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventoryEntry {
    pub id: String,
    pub source: String,
    pub app_id: Option<String>,
    pub name: String,
    pub nas_port: Option<u16>,
    pub fn_domain: Option<String>,
    pub upstream: Option<String>,
    pub path: Option<String>,
    pub status: String,
    pub reason: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InventorySource {
    pub id: String,
    pub name: String,
    pub status: String,
    pub count: Option<usize>,
    pub message: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerPortRow {
    pub id: String,
    pub container_id: String,
    pub container_name: String,
    pub state: String,
    pub protocol: String,
    pub host_ip: Option<String>,
    pub nas_port: Option<u16>,
    pub container_port: Option<u16>,
    pub upstream: Option<String>,
    pub fn_domain: Option<String>,
    pub status: String,
    pub reason: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DockerPortInventory {
    pub containers: usize,
    pub published_ports: usize,
    pub mapped_ports: usize,
    pub unmapped_ports: usize,
    pub unconfirmed_ports: usize,
    pub registry_available: bool,
    pub rows: Vec<DockerPortRow>,
    pub scope: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInventory {
    pub docker: Option<DockerPortInventory>,
    pub sources: Vec<InventorySource>,
    pub total_entries: usize,
    pub mapped_entries: usize,
    pub unmapped_entries: usize,
    pub services: Vec<DiscoveredService>,
    pub entries: Vec<InventoryEntry>,
    pub scope: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Bootstrap {
    pub allow_lan_access: bool,
    pub profiles: Vec<SavedProfile>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedProfile {
    pub profile: Profile,
    pub has_saved_password: bool,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectInput {
    pub fn_id: String,
    pub username: String,
    pub password: Option<String>,
    pub otp: Option<String>,
    #[serde(default)]
    pub remember: bool,
}
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionInfo {
    pub connected: bool,
    pub fn_id: String,
    pub username: String,
    pub relay: String,
    pub auth_mode: String,
    pub message: String,
}
#[derive(Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ProxyStatus {
    pub running: bool,
    pub listeners: Vec<ListenerInfo>,
    pub requests: u64,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ListenerInfo {
    pub name: String,
    pub local_url: String,
    pub upstream: String,
    pub nas_port: u16,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub connections: Vec<ConnectionSnapshot>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionSnapshot {
    pub id: String,
    pub services: Vec<ServiceRoute>,
    pub connection: ConnectionInfo,
    pub proxy: ProxyStatus,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteProbe {
    pub status: u16,
    pub reachable: bool,
    pub message: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LogEntry {
    pub time: u64,
    pub level: String,
    pub message: String,
}

pub fn normalize_fnid(input: &str) -> Result<String> {
    let id = input.trim().to_ascii_lowercase();
    let re = regex::Regex::new(r"^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$").unwrap();
    if !re.is_match(&id) || id.contains("--") {
        return Err(error("FN ID 只能包含字母、数字和单个连字符"));
    }
    Ok(id)
}
pub fn validate_upstream(input: &str, fn_id: &str) -> Result<url::Url> {
    let url = url::Url::parse(input.trim()).map_err(|_| error("请输入完整的 HTTPS 服务地址"))?;
    let suffix = format!(".{fn_id}.fnos.net");
    let host = url.host_str().unwrap_or_default();
    if url.scheme() != "https"
        || !host.ends_with(&suffix)
        || url.username() != ""
        || url.password().is_some()
        || url.port().is_some_and(|p| p != 443)
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(error("服务地址必须属于当前 NAS 的 FN Connect 子域名，使用 HTTPS，且不能包含凭据、查询参数或片段"));
    }
    if !host[..host.len() - suffix.len()].split('.').all(|label| {
        !label.is_empty()
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    }) {
        return Err(error("服务子域名格式无效"));
    }
    if url.path() != "/" && !url.path().is_empty() {
        return Err(error("请填写服务域名根地址；API 路径在访问本地代理时追加"));
    }
    Ok(url)
}
pub fn validate_routes(routes: &[ServiceRoute], fn_id: &str) -> Result<()> {
    let mut ports = std::collections::HashSet::new();
    if routes.len() > 32 {
        return Err(error("最多配置 32 个服务"));
    }
    for route in routes.iter().filter(|s| s.enabled) {
        if route.local_port == 0 || route.nas_port == 0 {
            return Err(error("端口需为 1–65535，不能为 0"));
        }
        if !ports.insert(route.local_port) {
            return Err(error("多个服务不能使用相同本地端口"));
        }
        validate_upstream(&route.upstream, fn_id)?;
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fnid_validation() {
        assert_eq!(normalize_fnid(" My-Nas ").unwrap(), "my-nas");
        assert!(normalize_fnid("https://evil.test").is_err());
    }
    #[test]
    fn upstream_rejects_ssrf_and_suffix_confusion() {
        assert!(validate_upstream("https://hash-0.my-nas.fnos.net/", "my-nas").is_ok());
        for s in [
            "https://my-nas.fnos.net.evil.test/",
            "http://hash.my-nas.fnos.net/",
            "https://127.0.0.1/",
            "https://hash.my-nas.fnos.net/?token=secret",
            "https://user@hash.my-nas.fnos.net/",
            "https://hash.other-nas.fnos.net/",
        ] {
            assert!(validate_upstream(s, "my-nas").is_err(), "{s}");
        }
    }
}
