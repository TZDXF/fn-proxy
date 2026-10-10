#[cfg(test)]
use crate::docker::{ContainerCollector, ContainerMetadata, StreamLimits, StreamMatch};
use crate::{
    error::{error, error_with, AppError, Result},
    resolver::{browser_client, millis, BROWSER_UA},
    text::Text,
    types::ConnectionInfo,
};
use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use futures_util::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use rand::{distributions::Alphanumeric, Rng};
use reqwest::cookie::{CookieStore, Jar};
use rsa::{pkcs8::DecodePublicKey, Pkcs1v15Encrypt, RsaPublicKey};
use serde_json::{json, Value};
use sha2::Sha256;
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::{Mutex, RwLock},
};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
use zeroize::{Zeroize, Zeroizing};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;
pub struct CryptoContext {
    key: Zeroizing<Vec<u8>>,
    iv: [u8; 16],
    rsa_key: String,
}
impl CryptoContext {
    pub fn new(public_key: &str) -> Result<Self> {
        let key: Vec<u8> = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .take(32)
            .collect();
        let mut iv = [0u8; 16];
        rand::thread_rng().fill(&mut iv);
        let public = RsaPublicKey::from_public_key_pem(public_key)
            .map_err(|_| error("auth.rsaPublicKeyUnsupported"))?;
        let wrapped = public
            .encrypt(&mut rand::thread_rng(), Pkcs1v15Encrypt, &key)
            .map_err(|_| error("auth.rsaWrapFailed"))?;
        Ok(Self {
            key: Zeroizing::new(key),
            iv,
            rsa_key: B64.encode(wrapped),
        })
    }
    pub fn encrypt(&self, payload: &Value) -> Result<Value> {
        let mut plain = serde_json::to_vec(payload)?;
        let cipher = cbc::Encryptor::<aes::Aes256>::new_from_slices(&self.key, &self.iv)
            .map_err(|_| error("auth.aesParams"))?
            .encrypt_padded_vec_mut::<Pkcs7>(&plain);
        plain.zeroize();
        Ok(
            json!({"req":"encrypted","iv":B64.encode(self.iv),"rsa":self.rsa_key,"aes":B64.encode(cipher)}),
        )
    }
    pub fn decrypt_secret(&self, value: &str) -> Result<Zeroizing<Vec<u8>>> {
        let ciphertext = B64
            .decode(value)
            .map_err(|_| error("auth.secretEncoding"))?;
        let plain = cbc::Decryptor::<aes::Aes256>::new_from_slices(&self.key, &self.iv)
            .map_err(|_| error("auth.aesParams"))?
            .decrypt_padded_vec_mut::<Pkcs7>(&ciphertext)
            .map_err(|_| error("auth.secretDecrypt"))?;
        Ok(Zeroizing::new(plain))
    }
}
pub fn signed_message(payload: &Value, secret: &[u8]) -> Result<String> {
    let text = serde_json::to_string(payload)?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret).map_err(|_| error("auth.signFailed"))?;
    mac.update(text.as_bytes());
    Ok(format!(
        "{}{}",
        B64.encode(mac.finalize().into_bytes()),
        text
    ))
}
pub struct RpcClient {
    socket: Socket,
    si: String,
    crypto: Option<CryptoContext>,
    secret: Zeroizing<Vec<u8>>,
    next_id: u64,
}
impl RpcClient {
    pub async fn open(base: &url::Url, jar: &Jar) -> Result<Self> {
        let mut url = base
            .join("/websocket?type=main")
            .map_err(|_| error("auth.wsUrlInvalid"))?;
        url.set_scheme(if base.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| error("auth.wsSchemeInvalid"))?;
        let mut request = url.as_str().into_client_request()?;
        request
            .headers_mut()
            .insert("User-Agent", BROWSER_UA.parse().unwrap());
        request.headers_mut().insert(
            "Origin",
            base.origin().ascii_serialization().parse().unwrap(),
        );
        if let Some(cookie) = jar.cookies(base) {
            request.headers_mut().insert("Cookie", cookie);
        }
        let (socket, _) =
            tokio::time::timeout(Duration::from_secs(20), connect_async(request)).await??;
        Ok(Self {
            socket,
            si: String::new(),
            crypto: None,
            secret: Zeroizing::new(vec![]),
            next_id: millis(),
        })
    }
    async fn receive(&mut self, id: &str) -> Result<Value> {
        tokio::time::timeout(Duration::from_secs(25), async {
            while let Some(message) = self.socket.next().await {
                match message? {
                    Message::Text(text) => {
                        let value: Value = serde_json::from_str(&text)?;
                        if value["reqid"].as_str() != Some(id) {
                            continue;
                        }
                        if value["result"].as_str() == Some("fail")
                            || value["errno"].as_i64().is_some_and(|e| e != 0)
                        {
                            let code = value["errno"]
                                .as_i64()
                                .or_else(|| value["code"].as_i64())
                                .unwrap_or(-1);
                            return Err(error_with(
                                "auth.nasRejected",
                                [("code", code.to_string())],
                            ));
                        }
                        return Ok(value);
                    }
                    Message::Ping(bytes) => {
                        self.socket.send(Message::Pong(bytes)).await?;
                    }
                    Message::Close(frame) => {
                        return Err(error_with(
                            "auth.authClosed",
                            [(
                                "closeCode",
                                frame
                                    .map(|f| u16::from(f.code).to_string())
                                    .unwrap_or_default(),
                            )],
                        ))
                    }
                    _ => {}
                }
            }
            Err(error("auth.authDisconnected"))
        })
        .await?
    }
    async fn send_request(
        &mut self,
        method: &str,
        mut arguments: Value,
        encrypted: bool,
    ) -> Result<String> {
        self.next_id += 1;
        let id = self.next_id.to_string();
        arguments["req"] = json!(method);
        arguments["reqid"] = json!(id);
        let wire = if encrypted {
            arguments["si"] = json!(self.si);
            serde_json::to_string(
                &self
                    .crypto
                    .as_ref()
                    .ok_or_else(|| error("auth.handshakeIncomplete"))?
                    .encrypt(&arguments)?,
            )?
        } else if self.secret.is_empty() {
            serde_json::to_string(&arguments)?
        } else {
            signed_message(&arguments, &self.secret)?
        };
        self.socket.send(Message::Text(wire.into())).await?;
        Ok(id)
    }
    pub async fn call(&mut self, method: &str, arguments: Value, encrypted: bool) -> Result<Value> {
        let id = self.send_request(method, arguments, encrypted).await?;
        self.receive(&id).await
    }
    #[cfg(test)]
    async fn list_containers_with_limits(
        &mut self,
        limits: StreamLimits,
    ) -> Result<Vec<ContainerMetadata>> {
        let id = self
            .send_request("appcgi.dockermgr.containerList", json!({"all":true}), false)
            .await?;
        tokio::time::timeout(limits.total, async {
            let mut collector = ContainerCollector::new(limits);
            let mut packets = 0;
            loop {
                let message = tokio::time::timeout(limits.idle, self.socket.next())
                    .await
                    .map_err(|_| error("containers.idleTimeout"))?
                    .ok_or_else(|| error("containers.disconnected"))??;
                match message {
                    Message::Text(text) => {
                        packets += 1;
                        if packets > limits.packets || text.len() > limits.frame_bytes {
                            return Err(error("containers.overflow"));
                        }
                        // First parse only reqid; unrelated packets need not match the container schema.
                        let header: StreamMatch = serde_json::from_str(&text)
                            .map_err(|_| error("containers.invalidJson"))?;
                        if header.reqid.as_ref().and_then(Value::as_str) != Some(id.as_str()) {
                            continue;
                        }
                        let packet = crate::docker::parse_container_packet(&text)?;
                        if collector.push(packet)? {
                            return collector.finish();
                        }
                    }
                    Message::Ping(bytes) => self.socket.send(Message::Pong(bytes)).await?,
                    Message::Close(_) => return Err(error("containers.closedEarly")),
                    _ => {}
                }
            }
        })
        .await
        .map_err(|_| error("containers.totalTimeout"))?
    }
    pub async fn heartbeat(&mut self) -> Result<()> {
        tokio::time::timeout(Duration::from_secs(10), async {
            self.socket
                .send(Message::Text("{\"req\":\"ping\"}".into()))
                .await?;
            while let Some(message) = self.socket.next().await {
                match message? {
                    Message::Text(t) => {
                        if serde_json::from_str::<Value>(&t)
                            .ok()
                            .is_some_and(|v| v["res"] == "pong")
                        {
                            return Ok(());
                        }
                    }
                    Message::Ping(b) => self.socket.send(Message::Pong(b)).await?,
                    Message::Close(frame) => {
                        return Err(error_with(
                            "auth.authClosed",
                            [(
                                "closeCode",
                                frame
                                    .map(|f| u16::from(f.code).to_string())
                                    .unwrap_or_default(),
                            )],
                        ))
                    }
                    _ => {}
                }
            }
            Err(error("auth.authDisconnected"))
        })
        .await?
    }
    pub async fn close(&mut self) {
        let _ = self.socket.close(None).await;
    }
}
pub struct NasSession {
    pub rpc: Mutex<RpcClient>,
    pub entry_token: RwLock<Zeroizing<String>>,
    pub info: ConnectionInfo,
    pub created_at: tokio::time::Instant,
    pub fn_connect: crate::fn_connect::FnConnectClient,
}
impl NasSession {
    pub async fn login(
        base: url::Url,
        fn_id: &str,
        username: &str,
        password: &str,
        otp: Option<&str>,
        remember: bool,
    ) -> Result<Arc<Self>> {
        let jar = Arc::new(Jar::default());
        let http = browser_client(jar.clone())?;
        // FN Connect first sets its HttpOnly `mode` cookie and redirects back to the same URL.
        let boot = http
            .get(base.join("/login").unwrap())
            .header(
                "Accept",
                "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
            )
            .header("Sec-Fetch-Mode", "navigate")
            .header("Sec-Fetch-Dest", "document")
            .header("Sec-Fetch-Site", "same-site")
            .header("Referer", "https://fnos.net/")
            .send()
            .await
            .map_err(|e| AppError::from(e).at("entry_handshake"))?;
        if !boot.status().is_success() {
            return Err(error_with(
                "auth.entryHandshakeFailed",
                [("status", boot.status().as_u16().to_string())],
            )
            .at("entry_handshake"));
        }
        let mut rpc = RpcClient::open(&base, &jar)
            .await
            .map_err(|e| e.at("websocket_open"))?;
        let key = rpc
            .call("util.crypto.getRSAPub", json!({}), false)
            .await
            .map_err(|e| e.at("crypto_handshake"))?;
        rpc.si = key["si"]
            .as_str()
            .ok_or_else(|| error("auth.sessionIdMissing").at("crypto_handshake"))?
            .to_owned();
        rpc.crypto =
            Some(CryptoContext::new(key["pub"].as_str().ok_or_else(
                || error("auth.rsaKeyMissing").at("crypto_handshake"),
            )?)?);
        let did = format!(
            "fn-proxy-{}",
            hex::encode(sha2::Sha256::digest(
                format!("{fn_id}:{username}").as_bytes()
            ))
        );
        let mut logged=rpc.call("user.login",json!({"user":username,"password":password,"stay":remember,"deviceType":"Browser","deviceName":"Windows-FNProxy","did":did,"ver":2}),true).await.map_err(|e| e.at("user_login"))?;
        if logged["isTwofaEnforced"].as_bool() == Some(true)
            && logged["isBindTwofaSecret"].as_bool() != Some(true)
        {
            return Err(error("auth.tfaBindingRequired").at("two_factor"));
        }
        if logged["isBindTwofaSecret"].as_bool() == Some(true)
            && logged["isTrustedDevice"].as_bool() != Some(true)
        {
            let code = otp
                .filter(|s| s.len() == 6 && s.bytes().all(|b| b.is_ascii_digit()))
                .ok_or_else(|| error("auth.tfaRequired").at("two_factor"))?;
            logged=rpc.call("user.2fa.loginVerify",json!({"code":code,"isTrustedDevice":false,"accessToken":logged["accessToken"],"stay":u8::from(remember),"deviceType":"Browser","deviceName":"Windows-FNProxy","did":did,"ver":2}),true).await.map_err(|e| e.at("two_factor"))?;
        }
        let auth_mode = if let Some(ticket) = logged["ticket"].as_str().filter(|t| !t.is_empty()) {
            let response = http
                .post(base.join("/app/ticket").unwrap())
                .json(&json!({"ticket":ticket}))
                .send()
                .await
                .map_err(|e| AppError::from(e).at("ticket_exchange"))?;
            if !response.status().is_success() {
                return Err(error_with(
                    "auth.ticketExchangeFailed",
                    [("status", response.status().as_u16().to_string())],
                )
                .at("ticket_exchange"));
            }
            "ticket-cookie"
        } else if let Some(token) = logged["token"].as_str().filter(|t| !t.is_empty()) {
            jar.add_cookie_str(&format!("fnos-token={token}; Path=/; Secure"), &base);
            "legacy"
        } else {
            return Err(error("auth.noSession").at("user_login"));
        };
        let secret = logged["secret"]
            .as_str()
            .ok_or_else(|| error("auth.secretMissing").at("login_secret"))?;
        rpc.secret = rpc
            .crypto
            .as_ref()
            .unwrap()
            .decrypt_secret(secret)
            .map_err(|e| e.at("login_secret"))?;
        let fn_connect = crate::fn_connect::FnConnectClient::new(
            jar.clone(),
            &base,
            if auth_mode == "legacy" {
                logged["token"].as_str().filter(|token| !token.is_empty())
            } else {
                None
            },
        );
        for field in ["token", "longToken", "secret", "ticket", "accessToken"] {
            if let Some(Value::String(value)) = logged.get_mut(field) {
                value.zeroize();
            }
        }
        let entry = rpc
            .call(
                "appcgi.sac.entry.v1.exchangeEntryToken",
                json!({"data":{}}),
                false,
            )
            .await
            .map_err(|e| e.at("entry_exchange"))?;
        let entry_token = entry["data"]["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("auth.entryTokenMissing").at("entry_exchange"))?
            .to_owned();
        if entry_token
            .bytes()
            .any(|b| b == b';' || b == b'\r' || b == b'\n')
        {
            return Err(error("auth.credentialInvalid").at("entry_exchange"));
        }
        let info = ConnectionInfo {
            connected: true,
            fn_id: fn_id.to_owned(),
            username: username.to_owned(),
            relay: base.origin().ascii_serialization(),
            auth_mode: auth_mode.to_owned(),
            fn_connect: None,
            message: Text::new("auth.loggedIn"),
        };
        Ok(Arc::new(Self {
            rpc: Mutex::new(rpc),
            fn_connect,
            entry_token: RwLock::new(Zeroizing::new(entry_token)),
            info,
            created_at: tokio::time::Instant::now(),
        }))
    }
    // Domain maintenance only needs the two entry registries, not container enumeration.
    pub async fn domain_inventory(&self) -> Result<crate::types::ServiceInventory> {
        let mut rpc = self.rpc.lock().await;
        let desktop = rpc
            .call(
                "appcgi.sac.entry.v1.getEntryList",
                json!({"data":{"language":"zh_CN"}}),
                false,
            )
            .await
            .and_then(|value| {
                crate::inventory::parse_relay_inventory(
                    &value,
                    &self.info.fn_id,
                    &self.info.relay,
                    false,
                )
            });
        let docker = rpc
            .call("appcgi.sac.entry.v1.dockerList", json!({}), false)
            .await
            .and_then(|value| {
                crate::inventory::parse_relay_inventory(
                    &value,
                    &self.info.fn_id,
                    &self.info.relay,
                    true,
                )
            });
        crate::inventory::merge_inventories(desktop, docker)
    }
    pub async fn inventory(&self) -> Result<crate::types::ServiceInventory> {
        // Only registered desktop/Docker entries are useful for remote HTTP mappings.
        // Do not enumerate containers when reading services.
        self.domain_inventory().await
    }
    pub async fn refresh_entry_token(&self) -> Result<()> {
        let mut rpc = self.rpc.lock().await;
        let previous = self.entry_token.read().await.clone();
        let result = rpc
            .call(
                "appcgi.sac.entry.v1.exchangeEntryToken",
                json!({"data":{"token":previous.as_str()}}),
                false,
            )
            .await;
        // A rejected old entry token is not evidence of a bad NAS password. An authenticated
        // socket may exchange a fresh token using the same empty request as initial login.
        let result = match result {
            Err(e) if e.text().code == "auth.nasRejected" => {
                rpc.call(
                    "appcgi.sac.entry.v1.exchangeEntryToken",
                    json!({"data":{}}),
                    false,
                )
                .await
            }
            other => other,
        }
        .map_err(|e| e.at("entry_refresh"))?;
        let token = result["data"]["token"]
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| error("auth.entryTokenRefreshFailed").at("entry_refresh"))?;
        if token.contains([';', '\r', '\n']) {
            return Err(error("auth.credentialRefreshInvalid").at("entry_refresh"));
        }
        *self.entry_token.write().await = Zeroizing::new(token.to_owned());
        Ok(())
    }
}
use sha2::Digest;
#[cfg(test)]
pub fn parse_entries(value: &Value, fn_id: &str) -> Vec<crate::types::DiscoveredService> {
    crate::inventory::parse_inventory(value, fn_id)
        .map(|report| report.services)
        .unwrap_or_default()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signed_messages_use_base64_hmac_and_exact_json() {
        let payload = json!({"req":"ping"});
        let wire = signed_message(&payload, b"secret").unwrap();
        assert_eq!(&wire[44..], serde_json::to_string(&payload).unwrap());
        assert_eq!(B64.decode(&wire[..44]).unwrap().len(), 32);
    }
    #[test]
    fn mapping_comes_from_server_not_hostname_decoding() {
        let entries = json!({"data":{"list":[{"entryKey":"app:test","title":"Test","uri":{"port":"8084","fnDomain":"b15c9af27e75-0"}},{"uri":{"port":80,"fnDomain":"evil.test/"}},{"uri":{"port":1234}}]}});
        let parsed = parse_entries(&entries, "my-nas");
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].nas_port, 8084);
        assert_eq!(
            parsed[0].upstream,
            "https://b15c9af27e75-0.my-nas.fnos.net/"
        );
    }
}

#[cfg(test)]
#[path = "auth_tests.rs"]
pub(crate) mod integration_tests;
