use super::*;
use crate::auth::integration_tests::mock_server;
use axum::{extract::ws::WebSocketUpgrade, response::IntoResponse};
use serde_json::{json, Value};

struct Fixture {
    local: url::Url,
    hub: SessionHub,
    cancel: CancellationToken,
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
    let service_app = Router::new()
        .route("/echo", any(echo))
        .route("/ws", any(ws_echo))
        .route(
            "/redirect",
            any(|| async { (StatusCode::FOUND, [("location", "/echo?next=ok")]).into_response() }),
        );
    let service_task = tokio::spawn(async move {
        axum::serve(service, service_app).await.unwrap();
    });
    let local_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = local_listener.local_addr().unwrap().port();
    let hub = Arc::new(RwLock::new(Some(session)));
    let cancel = CancellationToken::new();
    let ctx = ProxyContext {
        session: hub.clone(),
        upstream,
        local_port: port,
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
        cancel,
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
    let handles = start(&[route(port)], "my-nas", hub.clone(), counter.clone())
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
    assert!(start(&[insecure], "my-nas", hub, counter).await.is_err());
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
    let existing = start(&[route(first_port)], "my-nas", hub.clone(), counter.clone())
        .await
        .unwrap();
    let old_cancel = existing[0].cancel.clone();
    let listeners = existing.iter().map(|h| h.info.clone()).collect::<Vec<_>>();
    let additions =
        additional_routes(&[route(first_port), route(second_port)], &listeners).unwrap();
    assert_eq!(additions.len(), 1);
    assert_eq!(additions[0].local_port, second_port);
    let added = start(&additions, "my-nas", hub, counter.clone())
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
    let old = start(&[route(first_port)], "my-nas", hub.clone(), counter.clone())
        .await
        .unwrap();
    let result = start(
        &[route(free_port), route(busy_port)],
        "my-nas",
        hub,
        counter.clone(),
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
fn running_routes_cannot_be_removed_changed_or_disabled() {
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
    assert!(additional_routes(&[], &listeners).is_err());
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
            ..original
        },
    ] {
        assert!(additional_routes(&[changed], &listeners).is_err());
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
