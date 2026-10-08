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

#[derive(Clone, Copy)]
enum MockStream {
    Complete,
    Denied,
    PartialFail,
    NoTerminal,
    Disconnect,
    Malformed,
    Oversized,
    SlowProgress,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum MockAuth {
    Normal,
    RejectOldEntry,
    TwoFactor,
    RotateEntry,
}
#[derive(Clone)]
struct MockNas {
    private: Arc<RsaPrivateKey>,
    ticket_mode: bool,
    docker_allowed: bool,
    stream_mode: MockStream,
    auth_mode: MockAuth,
    exchanges: Arc<std::sync::atomic::AtomicU64>,
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
    while let Some(Ok(message)) = socket.recv().await {
        let text = match message {
            MockMessage::Text(text) => text,
            MockMessage::Ping(bytes) => {
                if socket.send(MockMessage::Pong(bytes)).await.is_err() {
                    break;
                }
                continue;
            }
            MockMessage::Pong(_) => continue,
            MockMessage::Close(_) => break,
            _ => continue,
        };
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
        if payload["req"] == "appcgi.dockermgr.containerList" {
            assert!(authenticated);
            assert_eq!(payload["all"], true);
            assert!(payload.get("data").is_none());
            if !serve_container_stream(&mut socket, id, state.stream_mode).await {
                break;
            }
            continue;
        }
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
                    json!({"reqid":id,"result":"suc","ticket":if state.ticket_mode {"fixture-ticket"}else{""},"token":"fixture-legacy-token","secret":B64.encode(encrypted),"isTwofaEnforced":false,"isBindTwofaSecret":state.auth_mode == MockAuth::TwoFactor})
                }
            }
            "appcgi.sac.entry.v1.exchangeEntryToken" => {
                let previous = payload["data"]["token"].as_str();
                if state.auth_mode == MockAuth::RejectOldEntry && previous.is_some() {
                    json!({"reqid":id,"result":"fail","errno":12345})
                } else if state.auth_mode == MockAuth::RotateEntry {
                    let version = state.exchanges.load(std::sync::atomic::Ordering::SeqCst);
                    if let Some(previous) = previous {
                        assert_eq!(previous, format!("fixture-entry-token-{version}"));
                    }
                    let version = state
                        .exchanges
                        .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                        + 1;
                    json!({"reqid":id,"result":"suc","data":{"token":format!("fixture-entry-token-{version}")}})
                } else {
                    json!({"reqid":id,"result":"suc","data":{"token":"fixture-entry-token"}})
                }
            }
            "appcgi.sac.entry.v1.getEntryList" => {
                json!({"reqid":id,"result":"suc","data":{"list":[{"entryKey":"fixture-app","title":"Fixture service","uri":{"port":"8084","fnDomain":"fixture-0"}},{"entryKey":"fixture-local","title":"Local-only fixture","uri":{"port":"9090"}},{"entryKey":"fixture-builtin","title":"Builtin fixture","uri":{"path":"/app/fixture"}}]}})
            }
            "appcgi.sac.entry.v1.dockerList" => {
                assert!(
                    payload.get("data").is_none(),
                    "Docker iframe omits data on this request"
                );
                if state.docker_allowed {
                    json!({"reqid":id,"result":"suc","data":{"list":[
                        {"appID":"fixture-container","uri":{"port":"8084","fnDomain":"fixture-0"}},
                        {"appID":"fixture-container","uri":{"port":"3000","fnDomain":"fixture-docker-0"}},
                        {"appID":"fixture-local","uri":{"port":"5000"}}
                    ]}})
                } else {
                    json!({"reqid":id,"result":"fail","errno":9999})
                }
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
    mock_server_with_docker(ticket_mode, true).await
}
async fn mock_server_with_docker(
    ticket_mode: bool,
    docker_allowed: bool,
) -> (url::Url, tokio::task::JoinHandle<()>) {
    mock_server_with_stream(ticket_mode, docker_allowed, MockStream::Complete).await
}
async fn mock_server_with_stream(
    ticket_mode: bool,
    docker_allowed: bool,
    stream_mode: MockStream,
) -> (url::Url, tokio::task::JoinHandle<()>) {
    mock_server_with_auth(ticket_mode, docker_allowed, stream_mode, MockAuth::Normal).await
}
async fn mock_server_with_auth(
    ticket_mode: bool,
    docker_allowed: bool,
    stream_mode: MockStream,
    auth_mode: MockAuth,
) -> (url::Url, tokio::task::JoinHandle<()>) {
    let state = MockNas {
        private: Arc::new(RsaPrivateKey::new(&mut rand::thread_rng(), 1024).unwrap()),
        ticket_mode,
        docker_allowed,
        stream_mode,
        auth_mode,
        exchanges: Arc::new(std::sync::atomic::AtomicU64::new(0)),
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
    assert_eq!(inventory.total_entries, 6);
    assert_eq!(inventory.mapped_entries, 3);
    assert_eq!(inventory.unmapped_entries, 3);
    assert_eq!(inventory.entries[1].status, "no-domain");
    assert_eq!(inventory.entries[2].status, "no-port");
    assert_eq!(inventory.sources[1].status, "ok");
    assert_eq!(inventory.sources[1].count, Some(3));
    assert_eq!(inventory.services.len(), 2);
    let ports = inventory.docker.as_ref().unwrap();
    assert_eq!(ports.containers, 3);
    assert_eq!(ports.published_ports, 5);
    assert_eq!(ports.mapped_ports, 2);
    assert_eq!(ports.unmapped_ports, 2);
    assert_eq!(ports.rows.len(), 6);
    assert_eq!(ports.rows.last().unwrap().status, "no-domain");
    assert!(ports.rows.last().unwrap().upstream.is_none());
    let exported = serde_json::to_string(&inventory).unwrap();
    assert!(!exported.contains("fixture-secret-environment"));
    assert!(!exported.contains("env"));
    assert_eq!(inventory.sources[2].status, "ok");

    let services = session.domain_inventory().await.unwrap().services;
    assert_eq!(services[0].nas_port, 8084);
    assert_eq!(services[0].fn_domain, "fixture-0");
    session.refresh_entry_token().await.unwrap();
    session.rpc.lock().await.heartbeat().await.unwrap();
    session.rpc.lock().await.close().await;
    server.abort();
}
#[tokio::test]
async fn docker_permission_failure_retains_desktop_inventory_and_rpc_session() {
    let (base, server) = mock_server_with_docker(true, false).await;
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
    let report = session.inventory().await.unwrap();
    assert_eq!(report.total_entries, 3);
    assert_eq!(report.services.len(), 1);
    assert_eq!(report.sources[0].status, "ok");
    assert_eq!(report.sources[1].status, "unavailable");
    assert_eq!(report.sources[1].count, None);
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

async fn stream_json(socket: &mut WebSocket, value: Value) -> bool {
    socket
        .send(MockMessage::Text(value.to_string().into()))
        .await
        .is_ok()
}
async fn serve_container_stream(socket: &mut WebSocket, id: &Value, mode: MockStream) -> bool {
    if matches!(mode, MockStream::Denied) {
        return stream_json(socket, json!({"reqid":id,"result":"fail","errno":9999})).await;
    }
    if !stream_json(
        socket,
        json!({"reqid":"fixture-unrelated","result":"fail","errno":9999,"rsp":"not-an-array"}),
    )
    .await
    {
        return false;
    }
    if !stream_json(
        socket,
        json!({"reqid":id,"result":"doing","rsp":[{
            "id":"fixture-container-full-id","names":["/fixture web"],"state":"running",
            "env":["API_KEY=fixture-secret-environment"],
            "ports":[{"publicPort":8084,"privatePort":80,"type":"tcp","ip":"0.0.0.0"}]
        }]}),
    )
    .await
    {
        return false;
    }
    match mode {
        MockStream::PartialFail => {
            return stream_json(socket, json!({"reqid":id,"result":"fail","errno":9999})).await
        }
        MockStream::NoTerminal => return true,
        MockStream::Disconnect => {
            let _ = socket.send(MockMessage::Close(None)).await;
            return false;
        }
        MockStream::Malformed => {
            return stream_json(
                socket,
                json!({"reqid":id,"result":"succ","rsp":{"unexpected":"schema"}}),
            )
            .await
        }
        MockStream::Oversized => {
            return stream_json(
                socket,
                json!({"reqid":id,"result":"doing","padding":"x".repeat(4096)}),
            )
            .await
        }
        MockStream::SlowProgress => {
            for _ in 0..5 {
                tokio::time::sleep(Duration::from_millis(75)).await;
                if !stream_json(socket, json!({"reqid":id,"result":"doing","rsp":[]})).await {
                    return false;
                }
            }
            return stream_json(socket, json!({"reqid":id,"result":"succ"})).await;
        }
        _ => {}
    }
    if socket
        .send(MockMessage::Ping(b"fixture-heartbeat".to_vec().into()))
        .await
        .is_err()
    {
        return false;
    }
    if !stream_json(
        socket,
        json!({"reqid":id,"result":"doing","rsp":[
            {"id":"fixture-container-full-id","ports":[
                {"publicPort":8084,"privatePort":80,"type":"tcp","ip":"0.0.0.0"},
                {"publicPort":3000,"privatePort":3000,"type":"tcp"}
            ]},
            {"id":"fixture-local-full-id","names":["/fixture local"],"state":"exited","ports":[
                {"publicPort":5000,"privatePort":5000,"type":"tcp"},
                {"privatePort":5432,"type":"tcp"},
                {"publicPort":5353,"privatePort":5353,"type":"udp"}
            ]}
        ]}),
    )
    .await
    {
        return false;
    }
    stream_json(
        socket,
        json!({"reqid":id,"result":"succ","rsp":[
            {"id":"fixture-other-full-id","names":["/fixture other"],"state":"running","ports":[
                {"publicPort":8084,"privatePort":80,"type":"tcp","ip":"127.0.0.2"}
            ]}
        ]}),
    )
    .await
}
#[tokio::test]
async fn container_stream_errors_and_timeouts_never_return_partial_metadata() {
    for mode in [
        MockStream::Denied,
        MockStream::PartialFail,
        MockStream::NoTerminal,
        MockStream::Disconnect,
        MockStream::Malformed,
        MockStream::Oversized,
        MockStream::SlowProgress,
    ] {
        let (base, server) = mock_server_with_stream(true, true, mode).await;
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
        let limits = match mode {
            MockStream::NoTerminal => StreamLimits {
                total: Duration::from_secs(1),
                idle: Duration::from_millis(30),
                ..StreamLimits::default()
            },
            MockStream::Oversized => StreamLimits {
                frame_bytes: 512,
                ..StreamLimits::default()
            },
            MockStream::SlowProgress => StreamLimits {
                total: Duration::from_millis(100),
                idle: Duration::from_secs(1),
                ..StreamLimits::default()
            },
            _ => StreamLimits::default(),
        };
        let result = session
            .rpc
            .lock()
            .await
            .list_containers_with_limits(limits)
            .await;
        assert!(
            result.is_err(),
            "A failed/incomplete stream cannot become a port inventory"
        );
        let message = result.err().unwrap().to_string();
        assert!(!message.contains("fixture-secret-environment"));
        session.rpc.lock().await.close().await;
        server.abort();
    }
}
#[tokio::test]
async fn interrupted_container_source_preserves_registered_mappings_and_session() {
    let (base, server) = mock_server_with_stream(true, true, MockStream::PartialFail).await;
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
    let report = session.inventory().await.unwrap();
    assert_eq!(report.services.len(), 2);
    assert_eq!(report.total_entries, 6);
    assert!(report.docker.is_none());
    assert_eq!(report.sources[2].status, "unavailable");
    assert_eq!(report.sources[2].count, None);
    session.rpc.lock().await.heartbeat().await.unwrap();
    session.rpc.lock().await.close().await;
    server.abort();
}

#[tokio::test]
async fn rejected_old_entry_token_is_reexchanged_without_password_login() {
    let (base, server) =
        mock_server_with_auth(true, true, MockStream::Complete, MockAuth::RejectOldEntry).await;
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
    *session.entry_token.write().await = Zeroizing::new("fixture-expired-token".into());
    session.refresh_entry_token().await.unwrap();
    assert_eq!(
        session.entry_token.read().await.as_str(),
        "fixture-entry-token"
    );
    session.rpc.lock().await.heartbeat().await.unwrap();
    session.rpc.lock().await.close().await;
    server.abort();
}
#[tokio::test]
async fn concurrent_entry_refreshes_use_the_latest_token_snapshot() {
    let (base, server) =
        mock_server_with_auth(true, true, MockStream::Complete, MockAuth::RotateEntry).await;
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
    let (first, second) =
        tokio::join!(session.refresh_entry_token(), session.refresh_entry_token());
    first.unwrap();
    second.unwrap();
    assert_eq!(
        session.entry_token.read().await.as_str(),
        "fixture-entry-token-3"
    );
    session.rpc.lock().await.close().await;
    server.abort();
}
#[tokio::test]
async fn missing_two_factor_is_classified_as_manual_intervention() {
    let (base, server) =
        mock_server_with_auth(true, true, MockStream::Complete, MockAuth::TwoFactor).await;
    let e = NasSession::login(
        base,
        "my-nas",
        "fixture-user",
        "fixture-password",
        None,
        false,
    )
    .await
    .err()
    .unwrap();
    assert_eq!(e.text().code, "auth.tfaRequired");
    assert_eq!(e.stage(), "two_factor");
    assert!(!e.retryable());
    server.abort();
}
