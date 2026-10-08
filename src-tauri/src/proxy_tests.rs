use super::*;
use crate::auth::integration_tests::mock_server;
use axum::{extract::ws::WebSocketUpgrade, response::IntoResponse};
use serde_json::{json, Value};

struct Fixture {
    local: url::Url,
    hub: SessionHub,
    upstream: UpstreamHub,
    cancel: CancellationToken,
    recovery: Arc<Recovery>,
    recovery_events: Arc<std::sync::Mutex<Vec<Text>>>,
    entry_hits: Arc<AtomicU64>,
    tasks: Vec<JoinHandle<()>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.cancel.cancel();
        for task in &self.tasks {
            task.abort();
        }
    }
}
async fn echo(request: Request) -> Response {
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, 1024 * 1024).await.unwrap();
    let result = json!({
        "method":parts.method.as_str(),
        "uri":parts.uri.to_string(),
        "body":String::from_utf8(body.to_vec()).unwrap(),
        "authorization":parts.headers.get(header::AUTHORIZATION).and_then(|v|v.to_str().ok()),
        "cookie":parts.headers.get(header::COOKIE).and_then(|v|v.to_str().ok()),
        "origin":parts.headers.get(header::ORIGIN).and_then(|v|v.to_str().ok()),
        "referer":parts.headers.get(header::REFERER).and_then(|v|v.to_str().ok()),
        "acceptEncoding":parts.headers.get(header::ACCEPT_ENCODING).and_then(|v|v.to_str().ok()),
        "hop":parts.headers.get("x-hop").and_then(|v|v.to_str().ok()),
    });
    Response::builder()
        .status(201)
        .header(header::SET_COOKIE, "entry-token=never-forward; Path=/")
        .header(
            header::SET_COOKIE,
            "application=session; Domain=fixture.test; Secure; HttpOnly; SameSite=None; Path=/",
        )
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(result.to_string()))
        .unwrap()
}
// Simulate the relay rejecting an expired entry credential, independently of RPC health.
async fn entry_gate(headers: HeaderMap) -> Response {
    let authorized = headers
        .get(header::COOKIE)
        .and_then(|value| value.to_str().ok())
        == Some("entry-token=fixture-entry-token");
    let status = if authorized {
        StatusCode::OK
    } else {
        StatusCode::FORBIDDEN
    };
    (status, "fixture relay authorization result").into_response()
}
const RELAY_EXPIRED: &str =
    "<html><title>FN Connect</title>FN Connect 暂无权限访问该服务...</html>";
async fn protected_ws(headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
        != Some("entry-token=fixture-entry-token")
    {
        return (StatusCode::FORBIDDEN, RELAY_EXPIRED).into_response();
    }
    ws_echo(headers, ws).await
}
async fn ws_echo(headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    assert_eq!(
        headers.get(header::COOKIE).unwrap(),
        "entry-token=fixture-entry-token"
    );
    ws.protocols(["fixture-protocol"])
        .on_upgrade(|mut socket| async move {
            while let Some(Ok(message)) = socket.recv().await {
                if matches!(message, LocalMessage::Close(_)) {
                    break;
                }
                if socket.send(message).await.is_err() {
                    break;
                }
            }
        })
}
async fn fixture() -> Fixture {
    fixture_with_access(false).await
}
async fn fixture_with_access(allow_lan_access: bool) -> Fixture {
    let (base, authentication) = mock_server(true).await;
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
    let service = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let upstream = url::Url::parse(&format!("http://{}/", service.local_addr().unwrap())).unwrap();
    let entry_hits = Arc::new(AtomicU64::new(0));
    let gate_hits = entry_hits.clone();
    let permanent_hits = entry_hits.clone();
    let service_app = Router::new()
        .route("/echo", any(echo))
        .route("/entry-gate", any(entry_gate))
        .route(
            "/recoverable",
            any(move |headers: HeaderMap, body: String| {
                let hits = gate_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    if headers.get(header::COOKIE).and_then(|v| v.to_str().ok())
                        == Some("entry-token=fixture-entry-token")
                    {
                        (StatusCode::OK, body).into_response()
                    } else {
                        (StatusCode::FORBIDDEN, RELAY_EXPIRED).into_response()
                    }
                }
            }),
        )
        .route(
            "/permanent-relay-error",
            any(move || {
                let hits = permanent_hits.clone();
                async move {
                    hits.fetch_add(1, Ordering::SeqCst);
                    (StatusCode::FORBIDDEN, RELAY_EXPIRED).into_response()
                }
            }),
        )
        .route(
            "/app-denied",
            any(|| async {
                (
                    StatusCode::FORBIDDEN,
                    "fixture application permission denied",
                )
            }),
        )
        .route(
            "/large-denied",
            any(|| async {
                (
                    StatusCode::FORBIDDEN,
                    "x".repeat(ERROR_PREFIX_LIMIT * 4) + RELAY_EXPIRED,
                )
            }),
        )
        .route(
            "/slow-denied",
            any(|| async {
                let stream = futures_util::stream::unfold(0u8, |step| async move {
                    match step {
                        0 => Some((Ok::<_, std::io::Error>("fixture first chunk"), 1)),
                        1 => {
                            tokio::time::sleep(std::time::Duration::from_millis(2200)).await;
                            Some((Ok("fixture last chunk"), 2))
                        }
                        _ => None,
                    }
                });
                let mut response = Response::new(Body::from_stream(stream));
                *response.status_mut() = StatusCode::FORBIDDEN;
                response
            }),
        )
        .route("/protected-ws", any(protected_ws))
        .route("/ws", any(ws_echo))
        .route(
            "/redirect",
            any(|| async { (StatusCode::FOUND, [("location", "/echo?next=ok")]).into_response() }),
        );
    let service_task = tokio::spawn(async move {
        axum::serve(service, service_app).await.unwrap();
    });
    let local_listener = TcpListener::bind((listen_address(allow_lan_access), 0))
        .await
        .unwrap();
    let port = local_listener.local_addr().unwrap().port();
    let hub = Arc::new(RwLock::new(Some(session)));
    let cancel = CancellationToken::new();
    let upstream = Arc::new(StdRwLock::new(upstream));
    let recovery = Arc::new(Recovery::default());
    let recovery_events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let ctx = ProxyContext {
        session: hub.clone(),
        recovery: Some(recovery.clone()),
        upstream: upstream.clone(),
        local_port: port,
        allow_lan_access,
        requests: Arc::new(AtomicU64::new(0)),
        cancel: cancel.clone(),
        http: reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap(),
    };
    // Only the fixture uses an HTTP upstream. Production start() always validates HTTPS/FN ID.
    let app = Router::new().fallback(any(forward)).with_state(ctx);
    let stop = cancel.clone();
    let proxy_task = tokio::spawn(async move {
        axum::serve(local_listener, app)
            .with_graceful_shutdown(stop.cancelled_owned())
            .await
            .unwrap();
    });
    Fixture {
        local: url::Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap(),
        hub,
        upstream,
        cancel,
        recovery,
        recovery_events,
        entry_hits,
        tasks: vec![authentication, service_task, proxy_task],
    }
}
#[tokio::test]
async fn http_api_keeps_method_query_body_auth_and_filters_cookies() {
    let f = fixture().await;
    let client = reqwest::Client::new();
    let response = client
        .post(f.local.join("echo?name=a%2Fb&empty=").unwrap())
        .header(header::AUTHORIZATION, "Bearer fixture-application-token")
        .header(
            header::COOKIE,
            "application=owned; entry-token =attacker; fnos-token=private; mode=attacker",
        )
        .header(header::ORIGIN, f.local.origin().ascii_serialization())
        .header(header::REFERER, f.local.join("ui").unwrap().as_str())
        .header(header::CONNECTION, "x-hop")
        .header("x-hop", "remove-me")
        .body("fixture request body")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let cookies: Vec<_> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|s| s.to_str().unwrap())
        .collect();
    assert_eq!(
        cookies,
        vec!["application=session; HttpOnly; SameSite=Lax; Path=/"]
    );
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["method"], "POST");
    assert_eq!(result["uri"], "/echo?name=a%2Fb&empty=");
    assert_eq!(result["body"], "fixture request body");
    assert_eq!(result["authorization"], "Bearer fixture-application-token");
    assert_eq!(
        result["cookie"],
        "application=owned; entry-token=fixture-entry-token"
    );
    assert_eq!(result["acceptEncoding"], "identity");
    assert!(result["hop"].is_null());
    assert_ne!(result["origin"], f.local.origin().ascii_serialization());
    assert!(result["referer"].as_str().unwrap().ends_with("/ui"));
    let redirects = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let response = redirects
        .get(f.local.join("redirect").unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::LOCATION],
        f.local.join("echo?next=ok").unwrap().as_str()
    );
}
#[tokio::test]
async fn rejects_untrusted_hosts_origins_and_disconnected_sessions() {
    let f = fixture().await;
    let client = reqwest::Client::new();
    for (name, value) in [
        ("host", "evil.test:12345"),
        ("origin", "https://evil.test"),
        ("sec-fetch-site", "cross-site"),
    ] {
        let response = client
            .get(f.local.join("echo").unwrap())
            .header(name, value)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
    }
    *f.hub.write().await = None;
    assert_eq!(
        client
            .get(f.local.join("echo").unwrap())
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
}
#[tokio::test]
async fn expired_entry_credential_is_not_recovered_by_a_healthy_rpc_heartbeat() {
    let f = fixture().await;
    let client = reqwest::Client::new();
    let session = f.hub.read().await.clone().unwrap();
    let url = f.local.join("entry-gate").unwrap();
    assert_eq!(client.get(url.clone()).send().await.unwrap().status(), 200);

    // Credential validity can change while the authenticated control socket stays healthy.
    *session.entry_token.write().await =
        zeroize::Zeroizing::new("fixture-expired-entry-token".to_owned());
    session.rpc.lock().await.heartbeat().await.unwrap();
    for _ in 0..2 {
        let response = client.get(url.clone()).send().await.unwrap();
        assert_eq!(response.status(), 403);
        assert_eq!(
            response.text().await.unwrap(),
            "fixture relay authorization result"
        );
    }
    assert_eq!(
        session.entry_token.read().await.as_str(),
        "fixture-expired-entry-token"
    );

    // Explicit refresh recovers service access without replacing the RPC session.
    session.refresh_entry_token().await.unwrap();
    assert_eq!(client.get(url).send().await.unwrap().status(), 200);
}
#[tokio::test]
async fn cross_site_navigation_is_rejected_even_with_valid_host_and_session() {
    let f = fixture().await;
    let client = reqwest::Client::new();
    let url = f.local.join("entry-gate").unwrap();
    assert_eq!(client.get(url.clone()).send().await.unwrap().status(), 200);
    let response = client
        .get(url)
        .header("sec-fetch-site", "cross-site")
        .header("sec-fetch-mode", "navigate")
        .header("sec-fetch-dest", "document")
        .header("sec-fetch-user", "?1")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    let body: Value = response.json().await.unwrap();
    assert_eq!(body["code"], "hostNotAllowed");
    // The local denial does not mean that the upstream service credential is invalid.
    let session = f.hub.read().await.clone().unwrap();
    assert_eq!(
        session.entry_token.read().await.as_str(),
        "fixture-entry-token"
    );
}
#[tokio::test]
async fn websocket_transfers_text_binary_protocol_and_stops() {
    let f = fixture().await;
    let mut url = f.local.join("ws").unwrap();
    url.set_scheme("ws").unwrap();
    let mut request = url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "fixture-protocol".parse().unwrap(),
    );
    let (mut socket, response) = connect_async(request).await.unwrap();
    assert_eq!(
        response.headers()["sec-websocket-protocol"],
        "fixture-protocol"
    );
    socket
        .send(RemoteMessage::Text("fixture text".into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        RemoteMessage::Text("fixture text".into())
    );
    socket
        .send(RemoteMessage::Binary(vec![0, 1, 2, 255].into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        RemoteMessage::Binary(vec![0, 1, 2, 255].into())
    );
    f.cancel.cancel();
    let stopped = tokio::time::timeout(std::time::Duration::from_secs(2), socket.next()).await;
    assert!(
        stopped.is_ok(),
        "Stopping the proxy must end the active WebSocket"
    );
}
fn route(port: u16) -> ServiceRoute {
    ServiceRoute {
        id: format!("fixture-{port}"),
        name: "fixture service".into(),
        nas_port: 8084,
        local_port: port,
        upstream: "https://fixture-0.my-nas.fnos.net/".into(),
        enabled: true,
    }
}
#[tokio::test]
async fn port_conflict_never_leaves_partially_started_listeners() {
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = free.local_addr().unwrap().port();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let busy = occupied.local_addr().unwrap().port();
    drop(free);
    let result = start(
        &[route(port), route(busy)],
        "my-nas",
        Arc::new(RwLock::new(None)),
        Arc::new(AtomicU64::new(0)),
        false,
    )
    .await;
    assert!(result.is_err());
    let rebound = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    drop(rebound);
}
#[tokio::test]
async fn stop_releases_listener_and_http_upstreams_stay_forbidden() {
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let hub = Arc::new(RwLock::new(None));
    let counter = Arc::new(AtomicU64::new(0));
    let handles = start(
        &[route(port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
    )
    .await
    .unwrap();
    assert_eq!(
        reqwest::get(format!("http://127.0.0.1:{port}/"))
            .await
            .unwrap()
            .status(),
        401
    );
    stop(handles).await;
    let rebound = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, port))
        .await
        .unwrap();
    drop(rebound);
    let mut insecure = route(port);
    insecure.upstream = "http://127.0.0.1:8084/".into();
    assert!(start(&[insecure], "my-nas", hub, counter, false)
        .await
        .is_err());
}

#[tokio::test]
async fn incrementally_added_listener_preserves_existing_socket_and_counter() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_port = first.local_addr().unwrap().port();
    let second_port = second.local_addr().unwrap().port();
    drop((first, second));
    let hub = Arc::new(RwLock::new(None));
    let counter = Arc::new(AtomicU64::new(42));
    let existing = start(
        &[route(first_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
    )
    .await
    .unwrap();
    let old_cancel = existing[0].cancel.clone();
    let listeners = existing.iter().map(|h| h.info.clone()).collect::<Vec<_>>();
    let additions =
        additional_routes(&[route(first_port), route(second_port)], &listeners).unwrap();
    assert_eq!(additions.len(), 1);
    assert_eq!(additions[0].local_port, second_port);
    let added = start(&additions, "my-nas", hub, counter.clone(), false)
        .await
        .unwrap();
    assert!(!old_cancel.is_cancelled());
    assert_eq!(counter.load(Ordering::Relaxed), 42);
    for port in [first_port, second_port] {
        assert_eq!(
            reqwest::get(format!("http://127.0.0.1:{port}/"))
                .await
                .unwrap()
                .status(),
            401
        );
    }
    stop(added).await;
    assert!(!old_cancel.is_cancelled());
    assert_eq!(
        reqwest::get(format!("http://127.0.0.1:{first_port}/"))
            .await
            .unwrap()
            .status(),
        401
    );
    stop(existing).await;
}

#[tokio::test]
async fn incremental_port_conflict_keeps_old_listener_alive_and_releases_new_ports() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_port = first.local_addr().unwrap().port();
    let free_port = free.local_addr().unwrap().port();
    let busy_port = occupied.local_addr().unwrap().port();
    drop((first, free));
    let hub = Arc::new(RwLock::new(None));
    let counter = Arc::new(AtomicU64::new(9));
    let old = start(
        &[route(first_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
    )
    .await
    .unwrap();
    let result = start(
        &[route(free_port), route(busy_port)],
        "my-nas",
        hub,
        counter.clone(),
        false,
    )
    .await;
    assert!(result.is_err());
    assert!(!old[0].cancel.is_cancelled());
    assert_eq!(counter.load(Ordering::Relaxed), 9);
    assert!(
        TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, free_port))
            .await
            .is_ok()
    );
    assert_eq!(
        reqwest::get(format!("http://127.0.0.1:{first_port}/"))
            .await
            .unwrap()
            .status(),
        401
    );
    stop(old).await;
}

#[test]
fn running_route_updates_only_reserve_new_local_ports() {
    let original = route(18084);
    let listener = ListenerInfo {
        name: original.name.clone(),
        nas_port: original.nas_port,
        local_url: "http://127.0.0.1:18084/".to_owned(),
        upstream: original.upstream.clone(),
    };
    let listeners = vec![listener];
    assert!(
        additional_routes(std::slice::from_ref(&original), &listeners)
            .unwrap()
            .is_empty()
    );
    assert!(additional_routes(&[], &listeners).unwrap().is_empty());
    for changed in [
        ServiceRoute {
            enabled: false,
            ..original.clone()
        },
        ServiceRoute {
            local_port: 18085,
            ..original.clone()
        },
        ServiceRoute {
            upstream: "https://other.my-nas.fnos.net/".into(),
            ..original.clone()
        },
        ServiceRoute {
            name: "renamed".into(),
            ..original.clone()
        },
        ServiceRoute {
            nas_port: 9090,
            ..original.clone()
        },
    ] {
        let expected = if changed.enabled && changed.local_port != original.local_port {
            vec![changed.clone()]
        } else {
            vec![]
        };
        let additions = additional_routes(&[changed], &listeners).unwrap();
        assert_eq!(additions.len(), expected.len());
        if let Some(next) = additions.first() {
            assert_eq!(next.local_port, expected[0].local_port);
        }
    }
}

#[tokio::test]
async fn adding_a_listener_does_not_interrupt_an_existing_websocket() {
    let f = fixture().await;
    let mut ws_url = f.local.join("ws").unwrap();
    ws_url.set_scheme("ws").unwrap();
    let mut request = ws_url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "fixture-protocol".parse().unwrap(),
    );
    let (mut socket, _) = connect_async(request).await.unwrap();
    socket
        .send(RemoteMessage::Text("before add".into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        RemoteMessage::Text("before add".into())
    );
    let free = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = free.local_addr().unwrap().port();
    drop(free);
    let added = start(
        &[route(port)],
        "my-nas",
        f.hub.clone(),
        Arc::new(AtomicU64::new(0)),
        false,
    )
    .await
    .unwrap();
    socket
        .send(RemoteMessage::Text("after add".into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        RemoteMessage::Text("after add".into())
    );
    assert!(!f.cancel.is_cancelled());
    socket.close(None).await.unwrap();
    stop(added).await;
}

#[tokio::test]
async fn lan_http_requests_keep_credentials_and_redirects_on_the_requested_host() {
    let local_only = fixture().await;
    let lan = fixture_with_access(true).await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let blocked = client
        .get(local_only.local.join("echo").unwrap())
        .header(
            header::HOST,
            format!("192.168.1.10:{}", local_only.local.port().unwrap()),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(blocked.status(), 403);
    let host = format!("192.168.1.10:{}", lan.local.port().unwrap());
    let origin = format!("http://{host}");
    let response = client
        .post(lan.local.join("echo?lan=1").unwrap())
        .header(header::HOST, &host)
        .header(header::ORIGIN, &origin)
        .header(header::REFERER, format!("{origin}/page"))
        .header(header::COOKIE, "application=ok; entry-token=attacker")
        .body("LAN request")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let result: Value = response.json().await.unwrap();
    assert_eq!(result["body"], "LAN request");
    assert_eq!(
        result["cookie"],
        "application=ok; entry-token=fixture-entry-token"
    );
    assert!(result["referer"].as_str().unwrap().ends_with("/page"));
    assert!(!result["referer"].as_str().unwrap().contains("192.168.1.10"));
    let redirect = client
        .get(lan.local.join("redirect").unwrap())
        .header(header::HOST, &host)
        .send()
        .await
        .unwrap();
    assert_eq!(redirect.status(), 302);
    assert_eq!(
        redirect.headers()[header::LOCATION],
        format!("{origin}/echo?next=ok")
    );
    for (header_name, value) in [
        ("host", format!("evil.test:{}", lan.local.port().unwrap())),
        ("origin", "https://evil.test".to_owned()),
        ("sec-fetch-site", "cross-site".to_owned()),
    ] {
        assert_eq!(
            client
                .get(lan.local.join("echo").unwrap())
                .header(header::HOST, &host)
                .header(header_name, value)
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
}
#[tokio::test]
async fn lan_websocket_accepts_matching_origin_and_forwards_service_credentials() {
    let lan = fixture_with_access(true).await;
    let mut url = lan.local.join("ws").unwrap();
    url.set_scheme("ws").unwrap();
    let mut request = url.as_str().into_client_request().unwrap();
    let host = format!("192.168.1.10:{}", lan.local.port().unwrap());
    request
        .headers_mut()
        .insert(header::HOST, host.parse().unwrap());
    request
        .headers_mut()
        .insert(header::ORIGIN, format!("http://{host}").parse().unwrap());
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "fixture-protocol".parse().unwrap(),
    );
    let (mut socket, response) = connect_async(request).await.unwrap();
    assert_eq!(
        response.headers()["sec-websocket-protocol"],
        "fixture-protocol"
    );
    socket
        .send(RemoteMessage::Text("LAN websocket".into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap(),
        RemoteMessage::Text("LAN websocket".into())
    );
    socket.close(None).await.unwrap();
}

#[tokio::test]
async fn live_domain_switch_preserves_listener_old_websocket_and_request_headers() {
    let mut f = fixture().await;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .unwrap();
    let before = client
        .get(f.local.join("echo").unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(before.status(), 201);
    let mut ws_url = f.local.join("ws").unwrap();
    ws_url.set_scheme("ws").unwrap();
    let (mut old_socket, _) = connect_async(ws_url.as_str()).await.unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let next = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let next_origin = next.origin().ascii_serialization();
    let redirect = next.join("echo?new=1").unwrap().to_string();
    let app = Router::new()
        .route("/echo", any(echo))
        .route("/ws", any(ws_echo))
        .route(
            "/redirect",
            any(move || {
                let redirect = redirect.clone();
                async move { (StatusCode::FOUND, [("location", redirect)]).into_response() }
            }),
        );
    f.tasks.push(tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    }));
    *f.upstream.write().unwrap() = next;
    // The old backend stops HTTP but retains the already established WebSocket task.
    f.tasks[1].abort();
    let response = client
        .post(f.local.join("echo?new=1").unwrap())
        .header(header::ORIGIN, f.local.origin().ascii_serialization())
        .header(header::REFERER, f.local.join("ui").unwrap().as_str())
        .body("after switch")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    let echoed: Value = response.json().await.unwrap();
    assert_eq!(echoed["origin"], next_origin);
    assert_eq!(echoed["referer"], format!("{next_origin}/ui"));
    assert_eq!(echoed["body"], "after switch");
    let response = client
        .get(f.local.join("redirect").unwrap())
        .send()
        .await
        .unwrap();
    assert_eq!(
        response.headers()[header::LOCATION],
        f.local.join("echo?new=1").unwrap().as_str()
    );
    old_socket
        .send(RemoteMessage::Text("still connected".into()))
        .await
        .unwrap();
    assert_eq!(
        old_socket.next().await.unwrap().unwrap(),
        RemoteMessage::Text("still connected".into())
    );
    let (mut new_socket, _) = connect_async(ws_url.as_str()).await.unwrap();
    new_socket
        .send(RemoteMessage::Text("new connection".into()))
        .await
        .unwrap();
    assert_eq!(
        new_socket.next().await.unwrap().unwrap(),
        RemoteMessage::Text("new connection".into())
    );
}

#[tokio::test]
async fn domain_update_is_validated_and_transactional_before_touching_live_targets() {
    let route = ServiceRoute {
        id: "fixed".into(),
        name: "API".into(),
        nas_port: 8084,
        local_port: 18084,
        upstream: "https://old-0.my-nas.fnos.net/".into(),
        enabled: true,
    };
    let cancel = CancellationToken::new();
    let mut handles = vec![ProxyHandle {
        route_id: route.id.clone(),
        upstream: Arc::new(StdRwLock::new(url::Url::parse(&route.upstream).unwrap())),
        info: ListenerInfo {
            name: route.name.clone(),
            local_url: "http://127.0.0.1:18084/".into(),
            upstream: route.upstream.clone(),
            nas_port: route.nas_port,
        },
        cancel: cancel.clone(),
        task: tokio::spawn(async {}),
    }];
    let next = ServiceRoute {
        upstream: "https://new-0.my-nas.fnos.net/".into(),
        ..route.clone()
    };
    assert!(
        update_upstreams(&mut handles, std::slice::from_ref(&next), "my-nas", || Err(
            error("save failed")
        ))
        .is_err()
    );
    assert_eq!(handles[0].info.upstream, route.upstream);
    assert_eq!(handles[0].upstream.read().unwrap().as_str(), route.upstream);
    let unsafe_route = ServiceRoute {
        upstream: "https://127.0.0.1/".into(),
        ..next.clone()
    };
    assert!(
        update_upstreams(&mut handles, &[unsafe_route], "my-nas", || panic!(
            "must validate before saving"
        ))
        .is_err()
    );
    let wrong_port = ServiceRoute {
        nas_port: 8085,
        ..next.clone()
    };
    assert!(
        update_upstreams(&mut handles, &[wrong_port], "my-nas", || panic!(
            "must reject port change before saving"
        ))
        .is_err()
    );
    update_upstreams(&mut handles, std::slice::from_ref(&next), "my-nas", || {
        Ok(())
    })
    .unwrap();
    assert_eq!(handles[0].info.upstream, next.upstream);
    assert_eq!(handles[0].upstream.read().unwrap().as_str(), next.upstream);
    assert_eq!(handles[0].info.local_url, "http://127.0.0.1:18084/");
    assert!(!cancel.is_cancelled());
    stop(handles).await;
}

#[tokio::test]
async fn live_edits_reuse_listener_and_update_all_forwarding_metadata() {
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = reserved.local_addr().unwrap().port();
    drop(reserved);
    let hub = Arc::new(RwLock::new(None));
    let counter = Arc::new(AtomicU64::new(42));
    let mut handles = start(
        &[route(port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
    )
    .await
    .unwrap();
    let upstream = handles[0].upstream.clone();
    let cancel = handles[0].cancel.clone();
    let task_id = handles[0].task.id();
    let next = ServiceRoute {
        name: "renamed".into(),
        nas_port: 9090,
        upstream: "https://new.my-nas.fnos.net/".into(),
        ..route(port)
    };
    apply_routes(
        &mut handles,
        std::slice::from_ref(&next),
        "my-nas",
        hub,
        counter.clone(),
        false,
        || Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].task.id(), task_id);
    assert!(Arc::ptr_eq(&handles[0].upstream, &upstream));
    assert!(!cancel.is_cancelled());
    assert_eq!(upstream.read().unwrap().as_str(), next.upstream);
    assert_eq!(handles[0].info.name, next.name);
    assert_eq!(handles[0].info.nas_port, next.nas_port);
    assert_eq!(handles[0].info.upstream, next.upstream);
    assert_eq!(counter.load(Ordering::Relaxed), 42);
    assert!(TcpListener::bind(("127.0.0.1", port)).await.is_err());
    stop(handles).await;
}

#[tokio::test]
async fn live_port_changes_and_deletions_release_only_replaced_listeners() {
    let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let second = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let third = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_port = first.local_addr().unwrap().port();
    let second_port = second.local_addr().unwrap().port();
    let third_port = third.local_addr().unwrap().port();
    drop((first, second, third));
    let hub = Arc::new(RwLock::new(None));
    let counter = Arc::new(AtomicU64::new(42));
    let mut handles = start(
        &[route(first_port), route(second_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
    )
    .await
    .unwrap();
    let first_cancel = handles[0].cancel.clone();
    let second_cancel = handles[1].cancel.clone();
    let second_task = handles[1].task.id();
    let moved = ServiceRoute {
        local_port: third_port,
        ..route(first_port)
    };
    apply_routes(
        &mut handles,
        &[moved, route(second_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
        || Ok(()),
    )
    .await
    .unwrap();
    assert!(first_cancel.is_cancelled());
    assert!(!second_cancel.is_cancelled());
    assert_eq!(handles[0].task.id(), second_task);
    assert!(TcpListener::bind(("127.0.0.1", first_port)).await.is_ok());
    assert!(TcpListener::bind(("127.0.0.1", third_port)).await.is_err());
    apply_routes(
        &mut handles,
        &[route(second_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
        || Ok(()),
    )
    .await
    .unwrap();
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].task.id(), second_task);
    assert!(TcpListener::bind(("127.0.0.1", third_port)).await.is_ok());
    let disabled = ServiceRoute {
        enabled: false,
        ..route(second_port)
    };
    apply_routes(
        &mut handles,
        &[disabled],
        "my-nas",
        hub,
        counter.clone(),
        false,
        || Ok(()),
    )
    .await
    .unwrap();
    assert!(handles.is_empty());
    assert!(second_cancel.is_cancelled());
    assert_eq!(counter.load(Ordering::Relaxed), 42);
    assert!(TcpListener::bind(("127.0.0.1", second_port)).await.is_ok());
}

#[tokio::test]
async fn live_update_failures_preserve_old_listener_and_release_prepared_ports() {
    let old = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let new = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let old_port = old.local_addr().unwrap().port();
    let new_port = new.local_addr().unwrap().port();
    let occupied_port = occupied.local_addr().unwrap().port();
    drop((old, new));
    let hub = Arc::new(RwLock::new(None));
    let counter = Arc::new(AtomicU64::new(42));
    let mut handles = start(
        &[route(old_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
    )
    .await
    .unwrap();
    let task_id = handles[0].task.id();
    let changed = ServiceRoute {
        name: "changed".into(),
        upstream: "https://new.my-nas.fnos.net/".into(),
        ..route(old_port)
    };
    assert!(apply_routes(
        &mut handles,
        &[changed.clone(), route(new_port), route(occupied_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
        || panic!("must bind before persisting")
    )
    .await
    .is_err());
    assert!(TcpListener::bind(("127.0.0.1", new_port)).await.is_ok());
    assert!(apply_routes(
        &mut handles,
        &[changed.clone(), route(new_port)],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
        || Err(error("storage failed"))
    )
    .await
    .is_err());
    assert!(TcpListener::bind(("127.0.0.1", new_port)).await.is_ok());
    assert!(apply_routes(
        &mut handles,
        &[],
        "my-nas",
        hub.clone(),
        counter.clone(),
        false,
        || Err(error("storage failed"))
    )
    .await
    .is_err());
    let unsafe_route = ServiceRoute {
        upstream: "https://127.0.0.1/".into(),
        ..changed
    };
    assert!(apply_routes(
        &mut handles,
        &[unsafe_route],
        "my-nas",
        hub,
        counter.clone(),
        false,
        || panic!("must validate before persisting")
    )
    .await
    .is_err());
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].task.id(), task_id);
    assert_eq!(handles[0].info.name, route(old_port).name);
    assert_eq!(handles[0].info.upstream, route(old_port).upstream);
    assert_eq!(
        handles[0].upstream.read().unwrap().as_str(),
        route(old_port).upstream
    );
    assert!(!handles[0].cancel.is_cancelled());
    assert_eq!(counter.load(Ordering::Relaxed), 42);
    assert!(TcpListener::bind(("127.0.0.1", old_port)).await.is_err());
    stop(handles).await;
}

async fn configure_fixture_recovery(f: &Fixture) -> Arc<AtomicU64> {
    let original = f.hub.read().await.clone().unwrap();
    let hub = f.hub.clone();
    let attempts = Arc::new(AtomicU64::new(0));
    let calls = attempts.clone();
    let events = f.recovery_events.clone();
    f.recovery.configure(
        Arc::new(move |_| {
            let hub = hub.clone();
            let original = original.clone();
            let calls = calls.clone();
            Box::pin(async move {
                calls.fetch_add(1, Ordering::SeqCst);
                original.refresh_entry_token().await?;
                *hub.write().await = Some(original);
                Ok(())
            })
        }),
        Arc::new(move |_, text| events.lock().unwrap().push(text)),
    );
    attempts
}
async fn expire_fixture(f: &Fixture) {
    let session = f.hub.read().await.clone().unwrap();
    *session.entry_token.write().await =
        zeroize::Zeroizing::new("fixture-expired-entry-token".to_owned());
}
#[tokio::test]
async fn fn_connect_expiry_refreshes_credentials_and_retries_empty_get_once() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    expire_fixture(&f).await;
    let response = reqwest::get(f.local.join("recoverable").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(f.entry_hits.load(Ordering::SeqCst), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let events = f.recovery_events.lock().unwrap();
    assert!(events.iter().any(|t| t.code == "logs.requestEntryExpired"));
    assert!(events.iter().any(|t| t.code == "logs.requestRetry"));
    let log = serde_json::to_string(&*events).unwrap();
    assert!(!log.contains("fixture-entry-token"));
    assert!(!log.contains("fixture-expired-entry-token"));
    assert!(!log.contains(RELAY_EXPIRED));
}
#[tokio::test]
async fn missing_session_recovers_before_post_is_sent_and_preserves_body() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    *f.hub.write().await = None;
    let response = reqwest::Client::new()
        .post(f.local.join("recoverable").unwrap())
        .body("fixture-write-body")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    assert_eq!(response.text().await.unwrap(), "fixture-write-body");
    assert_eq!(f.entry_hits.load(Ordering::SeqCst), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn expired_post_and_get_with_body_trigger_recovery_but_are_never_replayed() {
    for method in [axum::http::Method::POST, axum::http::Method::GET] {
        let f = fixture().await;
        let calls = configure_fixture_recovery(&f).await;
        expire_fixture(&f).await;
        let response = reqwest::Client::new()
            .request(method, f.local.join("recoverable").unwrap())
            .body("fixture-private-body")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 403);
        assert_eq!(response.text().await.unwrap(), RELAY_EXPIRED);
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let restored = f
                    .hub
                    .read()
                    .await
                    .clone()
                    .unwrap()
                    .entry_token
                    .read()
                    .await
                    .as_str()
                    == "fixture-entry-token";
                if restored {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(f.entry_hits.load(Ordering::SeqCst), 1);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let events = f.recovery_events.lock().unwrap();
        assert!(events.iter().any(|t| t.code == "logs.requestReplaySkipped"));
        assert!(!serde_json::to_string(&*events)
            .unwrap()
            .contains("fixture-private-body"));
    }
}
#[tokio::test]
async fn ordinary_app_denial_is_forwarded_unchanged_without_reauth() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    let response = reqwest::get(f.local.join("app-denied").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.text().await.unwrap(),
        "fixture application permission denied"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn repeated_relay_denial_does_not_loop_or_resubmit_requests_indefinitely() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    let response = reqwest::get(f.local.join("permanent-relay-error").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(response.text().await.unwrap(), RELAY_EXPIRED);
    assert_eq!(f.entry_hits.load(Ordering::SeqCst), 2);
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    // A new request cannot immediately start another recovery for the same failing page.
    assert_eq!(
        reqwest::get(f.local.join("permanent-relay-error").unwrap())
            .await
            .unwrap()
            .status(),
        503
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn host_origin_rejection_never_triggers_login() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    *f.hub.write().await = None;
    let response = reqwest::Client::new()
        .get(f.local.join("recoverable").unwrap())
        .header(header::ORIGIN, "https://fixture-untrusted.test")
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(f.entry_hits.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn bounded_and_timed_error_inspection_preserves_all_response_bytes() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    let response = reqwest::get(f.local.join("large-denied").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.text().await.unwrap(),
        "x".repeat(ERROR_PREFIX_LIMIT * 4) + RELAY_EXPIRED
    );
    let response = reqwest::get(f.local.join("slow-denied").unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), 403);
    assert_eq!(
        response.text().await.unwrap(),
        "fixture first chunkfixture last chunk"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
}
#[tokio::test]
async fn websocket_expired_handshake_recovers_before_local_upgrade() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    expire_fixture(&f).await;
    let mut url = f.local.join("protected-ws").unwrap();
    url.set_scheme("ws").unwrap();
    let mut request = url.as_str().into_client_request().unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        "fixture-protocol".parse().unwrap(),
    );
    let (mut socket, response) = connect_async(request).await.unwrap();
    assert_eq!(response.status(), 101);
    socket
        .send(RemoteMessage::Text("fixture recovered socket".into()))
        .await
        .unwrap();
    assert_eq!(
        socket.next().await.unwrap().unwrap().into_text().unwrap(),
        "fixture recovered socket"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
#[tokio::test]
async fn separate_connections_never_share_recovery_or_credentials() {
    let first = fixture().await;
    let second = fixture().await;
    let first_calls = configure_fixture_recovery(&first).await;
    let second_calls = configure_fixture_recovery(&second).await;
    expire_fixture(&first).await;
    expire_fixture(&second).await;
    assert_eq!(
        reqwest::get(first.local.join("recoverable").unwrap())
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(first_calls.load(Ordering::SeqCst), 1);
    assert_eq!(second_calls.load(Ordering::SeqCst), 0);
    let second_session = second.hub.read().await.clone().unwrap();
    assert_eq!(
        second_session.entry_token.read().await.as_str(),
        "fixture-expired-entry-token"
    );
}
#[tokio::test]
async fn concurrent_http_expiry_requests_share_one_credential_exchange() {
    let f = fixture().await;
    let calls = configure_fixture_recovery(&f).await;
    expire_fixture(&f).await;
    let client = reqwest::Client::new();
    let responses = futures_util::future::join_all(
        (0..16).map(|_| client.get(f.local.join("recoverable").unwrap()).send()),
    )
    .await;
    for response in responses {
        assert_eq!(response.unwrap().status(), 200);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}
