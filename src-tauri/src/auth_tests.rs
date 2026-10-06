use super::*;
use axum::{
    extract::{
        ws::{Message as MockMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    http::{header, HeaderMap},
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use rsa::{
    pkcs8::{EncodePublicKey, LineEnding},
    RsaPrivateKey,
};
use tokio::net::TcpListener;

#[derive(Clone)]
struct MockNas {
    private: Arc<RsaPrivateKey>,
    ticket_mode: bool,
}
const MOCK_SECRET: &[u8] = b"fixture-session-secret-not-a-real-credential";
async fn login_page(headers: HeaderMap) -> impl IntoResponse {
    if headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| s.contains("mode=fixture"))
    {
        (
            axum::http::StatusCode::OK,
            [("content-type", "text/html")],
            "<title>Fixture NAS</title>",
        )
            .into_response()
    } else {
        (
            axum::http::StatusCode::FOUND,
            [
                ("location", "/login"),
                ("set-cookie", "mode=fixture; Path=/; HttpOnly"),
            ],
            "",
        )
            .into_response()
    }
}
async fn ticket(Json(body): Json<Value>) -> impl IntoResponse {
    assert_eq!(body["ticket"], "fixture-ticket");
    (
        [("set-cookie", "nas-session=fixture; Path=/; HttpOnly")],
        Json(json!({"csrfToken":"fixture-csrf"})),
    )
}
async fn upgrade(
    State(state): State<MockNas>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> impl IntoResponse {
    assert!(headers
        .get(header::COOKIE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("mode=fixture"));
    ws.on_upgrade(move |socket| serve_mock(socket, state))
}
async fn serve_mock(mut socket: WebSocket, state: MockNas) {
    let mut authenticated = false;
    while let Some(Ok(MockMessage::Text(text))) = socket.recv().await {
        if text == "{\"req\":\"ping\"}" {
            socket
                .send(MockMessage::Text("{\"res\":\"pong\"}".into()))
                .await
                .unwrap();
            continue;
        }
        let payload: Value;
        let mut envelope_key = vec![];
        let mut envelope_iv = vec![];
        if text.starts_with('{') {
            let raw: Value = serde_json::from_str(&text).unwrap();
            if raw["req"] == "encrypted" {
                envelope_key = state
                    .private
                    .decrypt(
                        Pkcs1v15Encrypt,
                        &B64.decode(raw["rsa"].as_str().unwrap()).unwrap(),
                    )
                    .unwrap();
                envelope_iv = B64.decode(raw["iv"].as_str().unwrap()).unwrap();
                let cipher = B64.decode(raw["aes"].as_str().unwrap()).unwrap();
                let plain =
                    cbc::Decryptor::<aes::Aes256>::new_from_slices(&envelope_key, &envelope_iv)
                        .unwrap()
                        .decrypt_padded_vec_mut::<Pkcs7>(&cipher)
                        .unwrap();
                payload = serde_json::from_slice(&plain).unwrap();
                assert_eq!(payload["si"], "fixture-si");
            } else {
                payload = raw;
            }
        } else {
            assert!(authenticated);
            let mut mac = Hmac::<Sha256>::new_from_slice(MOCK_SECRET).unwrap();
            mac.update(text[44..].as_bytes());
            mac.verify_slice(&B64.decode(&text[..44]).unwrap()).unwrap();
            payload = serde_json::from_str(&text[44..]).unwrap();
        }
        let id = &payload["reqid"];
        let response = match payload["req"].as_str().unwrap() {
            "util.crypto.getRSAPub" => {
                json!({"reqid":id,"res":"util.crypto.getRSAPub","pub":state.private.to_public_key().to_public_key_pem(LineEnding::LF).unwrap(),"si":"fixture-si","result":"suc"})
            }
            "user.login" => {
                assert_eq!(payload["user"], "fixture-user");
                if payload["password"] != "fixture-password" {
                    json!({"reqid":id,"result":"fail","errno":1000})
                } else {
                    authenticated = true;
                    let encrypted =
                        cbc::Encryptor::<aes::Aes256>::new_from_slices(&envelope_key, &envelope_iv)
                            .unwrap()
                            .encrypt_padded_vec_mut::<Pkcs7>(MOCK_SECRET);
                    json!({"reqid":id,"result":"suc","ticket":if state.ticket_mode {"fixture-ticket"}else{""},"token":"fixture-legacy-token","secret":B64.encode(encrypted),"isTwofaEnforced":false,"isBindTwofaSecret":false})
                }
            }
            "appcgi.sac.entry.v1.exchangeEntryToken" => {
                json!({"reqid":id,"result":"suc","data":{"token":"fixture-entry-token"}})
            }
            "appcgi.sac.entry.v1.getEntryList" => {
                json!({"reqid":id,"result":"suc","data":{"list":[{"entryKey":"fixture-app","title":"Fixture service","uri":{"port":"8084","fnDomain":"fixture-0"}},{"entryKey":"fixture-local","title":"Local-only fixture","uri":{"port":"9090"}},{"entryKey":"fixture-builtin","title":"Builtin fixture","uri":{"path":"/app/fixture"}}]}})
            }
            method => panic!("unexpected mock RPC: {method}"),
        };
        socket
            .send(MockMessage::Text(response.to_string().into()))
            .await
            .unwrap();
    }
}
pub(crate) async fn mock_server(ticket_mode: bool) -> (url::Url, tokio::task::JoinHandle<()>) {
    let state = MockNas {
        private: Arc::new(RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap()),
        ticket_mode,
    };
    let app = Router::new()
        .route("/login", get(login_page))
        .route("/app/ticket", post(ticket))
        .route("/websocket", get(upgrade))
        .with_state(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url::Url::parse(&format!("http://{addr}/")).unwrap(), task)
}
#[tokio::test]
async fn full_ticket_login_encrypted_rpc_discovery_and_refresh() {
    let (base, server) = mock_server(true).await;
    let session = NasSession::login(
        base,
        "my-nas",
        "fixture-user",
        "fixture-password",
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(session.info.auth_mode, "ticket-cookie");
    assert_eq!(&**session.entry_token.read().await, "fixture-entry-token");
    let inventory = session.inventory().await.unwrap();
    assert_eq!(inventory.total_entries, 3);
    assert_eq!(inventory.mapped_entries, 1);
    assert_eq!(inventory.unmapped_entries, 2);
    assert_eq!(inventory.entries[1].status, "no-domain");
    assert_eq!(inventory.entries[2].status, "no-port");
    let services = session.discover().await.unwrap();
    assert_eq!(services[0].nas_port, 8084);
    assert_eq!(services[0].fn_domain, "fixture-0");
    session.refresh_entry_token().await.unwrap();
    session.rpc.lock().await.heartbeat().await.unwrap();
    session.rpc.lock().await.close().await;
    server.abort();
}
#[tokio::test]
async fn legacy_login_and_rejected_credentials_are_handled() {
    let (base, server) = mock_server(false).await;
    let result = NasSession::login(
        base.clone(),
        "my-nas",
        "fixture-user",
        "wrong-fixture-password",
        None,
        false,
    )
    .await;
    assert!(result.is_err());
    let message = result.err().unwrap().to_string();
    assert!(!message.contains("wrong-fixture-password"));
    let session = NasSession::login(
        base,
        "my-nas",
        "fixture-user",
        "fixture-password",
        None,
        false,
    )
    .await
    .unwrap();
    assert_eq!(session.info.auth_mode, "legacy");
    session.rpc.lock().await.close().await;
    server.abort();
}
#[tokio::test]
#[ignore = "Explicit opt-in: only a public handshake, never a password attempt"]
async fn live_fn_connect_handshake() {
    let id = std::env::var("FN_PROXY_TEST_ID").expect("set FN_PROXY_TEST_ID explicitly");
    let base = crate::resolver::resolve(&id).await.unwrap();
    let jar = Arc::new(Jar::default());
    let client = browser_client(jar.clone()).unwrap();
    let response = client
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
        .unwrap();
    assert!(response.status().is_success());
    let mut rpc = RpcClient::open(&base, &jar).await.unwrap();
    let public = rpc
        .call("util.crypto.getRSAPub", json!({}), false)
        .await
        .unwrap();
    assert!(public["pub"]
        .as_str()
        .is_some_and(|s| s.contains("PUBLIC KEY")));
    assert!(public["si"].is_string());
    CryptoContext::new(public["pub"].as_str().unwrap()).unwrap();
    rpc.close().await;
}
