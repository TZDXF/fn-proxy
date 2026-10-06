use crate::error::{error, Result};
use crate::types::normalize_fnid;
use md5::Md5;
use reqwest::{cookie::Jar, Client};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36";
pub fn millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
pub fn browser_client(jar: Arc<Jar>) -> Result<Client> {
    Ok(Client::builder()
        .cookie_provider(jar)
        .user_agent(BROWSER_UA)
        .connect_timeout(Duration::from_secs(12))
        .timeout(Duration::from_secs(25))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 8 {
                attempt.error("too many redirects")
            } else if attempt
                .previous()
                .first()
                .is_some_and(|u| u.host_str() != attempt.url().host_str())
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()?)
}
fn md5_hex(input: impl AsRef<[u8]>) -> String {
    hex::encode(Md5::digest(input.as_ref()))
}
pub fn resolver_headers(
    id: &str,
    timestamp: u64,
    nonce: &str,
    prefix: &str,
    key: &str,
) -> (String, String) {
    let body = serde_json::to_string(&json!({"fnId":id})).unwrap();
    let material = format!("trim_connect`{id}`{timestamp}`anna");
    let fn_sign = hex::encode(Sha256::digest(material.as_bytes()));
    let signature = md5_hex(format!(
        "{prefix}_/api/v1/fn/con_{nonce}_{timestamp}_{}_{}",
        md5_hex(body),
        key
    ));
    (
        fn_sign,
        format!("nonce={nonce}&timestamp={timestamp}&sign={signature}"),
    )
}
pub async fn resolve(id: &str) -> Result<url::Url> {
    let id = normalize_fnid(id)?;
    let client = browser_client(Arc::new(Jar::default()))?;
    let page = client
        .get("https://fnos.net/")
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let script =
        regex::Regex::new(r#"src="(https://static2\.fnnas\.com/connect/assets/[^\"]+\.js)""#)
            .unwrap()
            .captures(&page)
            .and_then(|c| c.get(1))
            .ok_or_else(|| error("FN Connect 入口已更新，未找到解析脚本"))?
            .as_str()
            .to_owned();
    let source = client
        .get(script)
        .send()
        .await?
        .error_for_status()?
        .text()
        .await?;
    let capture = |pattern: &str| -> Result<String> {
        regex::Regex::new(pattern)
            .unwrap()
            .captures(&source)
            .and_then(|c| c.get(1))
            .map(|m| m.as_str().to_owned())
            .ok_or_else(|| error("FN ID 解析签名格式已变化，请更新应用"))
    };
    let key = capture(r#"ApiKey="([^"]+)""#)?;
    let prefix = capture(r#""PREFIX","([^"]+)""#)?;
    let timestamp = millis();
    let nonce = rand::random::<u32>() % 900000 + 100000;
    let (fn_sign, authx) = resolver_headers(&id, timestamp, &nonce.to_string(), &prefix, &key);
    let response: Value = client
        .post("https://fnos.net/api/v1/fn/con")
        .header("fn-sign", fn_sign)
        .header("authx", authx)
        .json(&json!({"fnId":id}))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    if response["code"].as_i64() != Some(0) {
        return Err(error("FN ID 解析失败，请检查 FN ID 和 NAS 在线状态"));
    }
    let expected = format!("{id}.fnos.net");
    let relay = response["data"]["fn"]
        .as_array()
        .and_then(|list| {
            list.iter().filter_map(Value::as_str).find_map(|host| {
                let url = url::Url::parse(&format!("https://{host}/")).ok()?;
                (url.host_str() == Some(expected.as_str())
                    && url.port().is_none_or(|p| p == 443)
                    && url.username().is_empty()
                    && url.password().is_none())
                .then_some(url)
            })
        })
        .ok_or_else(|| {
            error("未找到当前 NAS 的 FN Connect 中继地址；本应用不会自动跳转到内网 IP")
        })?;
    Ok(relay)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_are_deterministic_and_timestamped() {
        let a = resolver_headers("my-nas", 123, "100000", "prefix", "public-key");
        assert_eq!(a.0.len(), 64);
        assert!(a.1.starts_with("nonce=100000&timestamp=123&sign="));
        assert_ne!(
            a,
            resolver_headers("my-nas", 124, "100000", "prefix", "public-key")
        );
    }
}
