//! Read-only FN Connect entitlement metadata from the NAS account API.
//! Never serialize the cloud account, NAS token, response body or request diagnostics.
use crate::text::Text;
use reqwest::{cookie::Jar, Client};
use serde::Serialize;
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use zeroize::Zeroizing;

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FnConnectEntitlement {
    pub tier: String,
    pub bandwidth_mbps: Option<f64>,
    pub traffic_used_mb: Option<f64>,
    pub traffic_per_month_mb: Option<f64>,
    pub end_time: Option<u64>,
}
#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct FnConnectInfo {
    pub status: String,
    pub entitlement: Option<FnConnectEntitlement>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
}
impl FnConnectInfo {
    fn unavailable(reason: Text) -> Self {
        Self {
            status: "unavailable".into(),
            entitlement: None,
            reason: Some(reason),
        }
    }
}
fn number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|n| n.is_finite() && *n >= 0.0)
}
pub fn parse_account_check(value: &Value) -> FnConnectInfo {
    match value["code"].as_i64() {
        Some(0) => {}
        Some(code) => {
            let reason = match code {
                401 | 13003 => "fnConnect.authRejected",
                13002 => "fnConnect.permissionDenied",
                _ => "fnConnect.accountRejected",
            };
            return FnConnectInfo::unavailable(Text::with(reason, [("code", code.to_string())]));
        }
        None => return FnConnectInfo::unavailable(Text::new("fnConnect.invalidResponse")),
    }
    let data = &value["data"];
    if data["status"].as_u64() == Some(2) {
        return FnConnectInfo {
            status: "unbound".into(),
            entitlement: None,
            reason: None,
        };
    }
    let entitlement = &data["connectEntitlement"];
    if data["status"].as_u64() != Some(1) || !entitlement.is_object() {
        return FnConnectInfo::unavailable(Text::new("fnConnect.entitlementMissing"));
    }
    let tier = match entitlement["type"].as_str() {
        Some("base") => "base",
        Some("premium") => "premium",
        Some("pro") => "pro",
        _ => "unknown",
    };
    FnConnectInfo {
        status: "available".into(),
        reason: None,
        entitlement: Some(FnConnectEntitlement {
            tier: tier.into(),
            bandwidth_mbps: number(&entitlement["bandwidth"]),
            traffic_used_mb: number(&entitlement["trafficUsed"]),
            traffic_per_month_mb: number(&entitlement["trafficPerMonth"]),
            // The NAS frontend converts endTime from seconds to milliseconds.
            end_time: entitlement["endTime"]
                .as_u64()
                .filter(|n| *n > 0 && *n <= 253_402_300_799),
        }),
    }
}

pub struct FnConnectClient {
    http: Option<Client>,
    endpoint: url::Url,
    token: Option<Zeroizing<String>>,
}
impl FnConnectClient {
    pub fn new(jar: Arc<Jar>, base: &url::Url, token: Option<&str>) -> Self {
        Self {
            // Share the authenticated NAS cookies, but never follow account API redirects:
            // even a same-host HTTP downgrade must not receive the legacy token.
            http: Client::builder()
                .cookie_provider(jar)
                .user_agent(crate::resolver::BROWSER_UA)
                .redirect(reqwest::redirect::Policy::none())
                .connect_timeout(Duration::from_secs(4))
                .timeout(Duration::from_secs(6))
                .build()
                .ok(),
            endpoint: base
                .join("/v1/accountapi/check")
                .expect("validated NAS base URL"),
            token: token.map(|token| Zeroizing::new(token.to_owned())),
        }
    }
    pub async fn read(&self) -> FnConnectInfo {
        // Use a NAS login token whenever one was issued, including alongside a
        // ticket. Cookie-only NAS versions omit it. Never send service entry-token.
        let Some(http) = &self.http else {
            return FnConnectInfo::unavailable(Text::new("fnConnect.clientUnavailable"));
        };
        let body = self
            .token
            .as_ref()
            .map_or_else(|| json!({}), |token| json!({"token": &**token}));
        let result = tokio::time::timeout(Duration::from_secs(6), async {
            let mut response = http
                .post(self.endpoint.clone())
                .json(&body)
                .send()
                .await
                .map_err(transport_reason)?;
            if !response.status().is_success() {
                return Err(Text::with(
                    "fnConnect.httpRejected",
                    [("status", response.status().as_u16().to_string())],
                ));
            }
            let mut bytes = Zeroizing::new(Vec::new());
            while let Some(chunk) = response.chunk().await.map_err(transport_reason)? {
                if bytes.len() + chunk.len() > 64 * 1024 {
                    return Err(Text::new("fnConnect.responseTooLarge"));
                }
                bytes.extend_from_slice(&chunk);
            }
            let value: Value = serde_json::from_slice(&bytes)
                .map_err(|_| Text::new("fnConnect.invalidResponse"))?;
            Ok(parse_account_check(&value))
        })
        .await;
        match result {
            Ok(Ok(info)) => info,
            Ok(Err(reason)) => FnConnectInfo::unavailable(reason),
            Err(_) => FnConnectInfo::unavailable(Text::new("fnConnect.timeout")),
        }
    }
}

// Allowlisted diagnostics only: reqwest errors may contain URLs or credentials.
fn transport_reason(error: reqwest::Error) -> Text {
    Text::new(if error.is_timeout() {
        "fnConnect.timeout"
    } else {
        "fnConnect.network"
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn account(tier: &str) -> Value {
        json!({"code":0,"data":{"status":1,"account":"fixture-cloud-account","connectEntitlement":{
            "type":tier,"bandwidth":12,"trafficUsed":1024,"trafficPerMonth":512000,"endTime":1936742400
        }}})
    }
    #[test]
    fn recognizes_all_tiers_and_preserves_server_units() {
        for tier in ["base", "premium", "pro"] {
            let info = parse_account_check(&account(tier));
            assert_eq!(info.status, "available");
            let e = info.entitlement.unwrap();
            assert_eq!(e.tier, tier);
            assert_eq!(e.bandwidth_mbps, Some(12.0));
            assert_eq!(e.traffic_used_mb, Some(1024.0));
            assert_eq!(e.traffic_per_month_mb, Some(512000.0));
            assert_eq!(e.end_time, Some(1936742400));
        }
        assert_eq!(
            parse_account_check(&account("future-tier"))
                .entitlement
                .unwrap()
                .tier,
            "unknown"
        );
    }
    #[test]
    fn missing_or_denied_metadata_is_not_a_base_plan() {
        for value in [
            json!({}),
            json!({"code":401}),
            json!({"code":0,"data":{"status":1}}),
            json!({"code":0,"data":{"status":1,"connectEntitlement":null}}),
        ] {
            assert_eq!(parse_account_check(&value).status, "unavailable");
        }
        assert_eq!(
            parse_account_check(&json!({"code":0,"data":{"status":2}})).status,
            "unbound"
        );
    }
    #[test]
    fn failures_preserve_only_allowlisted_reasons_and_numeric_codes() {
        for (code, expected) in [
            (13003, "fnConnect.authRejected"),
            (401, "fnConnect.authRejected"),
            (13002, "fnConnect.permissionDenied"),
            (13005, "fnConnect.accountRejected"),
        ] {
            let info = parse_account_check(&json!({
                "code":code,
                "msg":"fixture-private-response-token",
                "data":{"account":"fixture-private-account"}
            }));
            assert_eq!(info.status, "unavailable");
            let reason = info.reason.as_ref().unwrap();
            assert_eq!(reason.code, expected);
            assert_eq!(reason.params["code"], code.to_string());
            let wire = serde_json::to_string(&info).unwrap();
            assert!(!wire.contains("fixture-private"));
            assert!(!wire.contains("msg"));
        }
        let info = parse_account_check(&json!({"code":"fixture-private-code"}));
        assert_eq!(info.reason.unwrap().code, "fnConnect.invalidResponse");
        let info = parse_account_check(&json!({"code":0,"data":{"status":1}}));
        assert_eq!(info.reason.unwrap().code, "fnConnect.entitlementMissing");
        assert!(parse_account_check(&account("base")).reason.is_none());
        assert!(parse_account_check(&json!({"code":0,"data":{"status":2}}))
            .reason
            .is_none());
    }
    #[test]
    fn invalid_numbers_remain_unknown_and_zero_quota_means_unlimited() {
        let mut value = account("base");
        value["data"]["connectEntitlement"] = json!({"type":"base","bandwidth":-1,"trafficUsed":"1024","trafficPerMonth":0,"endTime":0});
        let e = parse_account_check(&value).entitlement.unwrap();
        assert_eq!(e.bandwidth_mbps, None);
        assert_eq!(e.traffic_used_mb, None);
        assert_eq!(e.traffic_per_month_mb, Some(0.0));
        assert_eq!(e.end_time, None);
        let serialized = serde_json::to_string(&parse_account_check(&account("premium"))).unwrap();
        assert!(!serialized.contains("fixture-cloud-account"));
    }
    #[tokio::test]
    async fn http_failures_malformed_or_oversized_bodies_and_redirects_are_unavailable() {
        use axum::{
            http::{header, StatusCode},
            response::IntoResponse,
            routing::post,
            Router,
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        for (status, body) in [
            (401, "denied".to_owned()),
            (500, "failed".to_owned()),
            (200, "<html>login</html>".to_owned()),
            (200, "x".repeat(65 * 1024)),
            (307, String::new()),
        ] {
            let oversized = body.len() > 64 * 1024;
            let leaked = Arc::new(AtomicUsize::new(0));
            let observed = leaked.clone();
            let app = Router::new()
                .route(
                    "/v1/accountapi/check",
                    post(move || {
                        let body = body.clone();
                        async move {
                            let mut response =
                                (StatusCode::from_u16(status).unwrap(), body).into_response();
                            response
                                .headers_mut()
                                .insert(header::LOCATION, "/leak".parse().unwrap());
                            response
                        }
                    }),
                )
                .route(
                    "/leak",
                    post(move || {
                        let observed = observed.clone();
                        async move {
                            observed.fetch_add(1, Ordering::SeqCst);
                            "must not be reached"
                        }
                    }),
                );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base =
                url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, app).await.unwrap();
            });
            let client =
                FnConnectClient::new(Arc::new(Jar::default()), &base, Some("fixture-nas-token"));
            let info = client.read().await;
            assert_eq!(info.status, "unavailable");
            let reason = info.reason.as_ref().unwrap();
            if status != 200 {
                assert_eq!(reason.code, "fnConnect.httpRejected");
                assert_eq!(reason.params["status"], status.to_string());
            } else {
                assert_eq!(
                    reason.code,
                    if oversized {
                        "fnConnect.responseTooLarge"
                    } else {
                        "fnConnect.invalidResponse"
                    }
                );
            }
            let wire = serde_json::to_string(&info).unwrap();
            assert!(!wire.contains("fixture-nas-token"));
            assert!(!wire.contains("http://"));
            assert_eq!(leaked.load(Ordering::SeqCst), 0);
            server.abort();
        }
    }
}
