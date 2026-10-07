use crate::{
    auth::NasSession,
    error::{error, error_with, Result},
    types::{validate_routes, validate_upstream, ListenerInfo, ServiceRoute},
};
use axum::{
    body::Body,
    extract::{
        ws::{Message as LocalMessage, WebSocket, WebSocketUpgrade},
        FromRequestParts, Request, State,
    },
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::Response,
    routing::any,
    Router,
};
use futures_util::{SinkExt, StreamExt};
use std::sync::{
    atomic::{AtomicU64, Ordering},
    Arc, RwLock as StdRwLock,
};
use tokio::{net::TcpListener, sync::RwLock, task::JoinHandle};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message as RemoteMessage},
};
use tokio_util::sync::CancellationToken;

pub type SessionHub = Arc<RwLock<Option<Arc<NasSession>>>>;
pub type UpstreamHub = Arc<StdRwLock<url::Url>>;
#[derive(Clone)]
pub struct ProxyContext {
    pub session: SessionHub,
    pub upstream: UpstreamHub,
    pub local_port: u16,
    pub allow_lan_access: bool,
    pub requests: Arc<AtomicU64>,
    pub cancel: CancellationToken,
    pub http: reqwest::Client,
}
pub struct ProxyHandle {
    pub route_id: String,
    pub upstream: UpstreamHub,
    pub info: ListenerInfo,
    pub cancel: CancellationToken,
    pub task: JoinHandle<()>,
}
/// Proxy error responses are consumed by arbitrary API clients in a browser,
/// so they carry a stable machine code plus a locale-independent message
/// instead of text from a specific UI locale.
fn fail(status: StatusCode, code: &str, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(
            serde_json::json!({"code":code,"error":message}).to_string(),
        ))
        .unwrap()
}
fn allowed_host(host: &str, port: u16, allow_lan_access: bool) -> bool {
    if host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}") {
        return true;
    }
    if !allow_lan_access {
        return false;
    }
    let Some((address, host_port)) = host.rsplit_once(':') else {
        return false;
    };
    if host_port != port.to_string() {
        return false;
    }
    address
        .parse::<std::net::Ipv4Addr>()
        .is_ok_and(|ip| ip.is_private() || ip.is_link_local())
}
pub fn is_allowed_request(headers: &HeaderMap, port: u16, allow_lan_access: bool) -> bool {
    // Ambiguous authorities/origins must not bypass validation through first-value lookup.
    if headers.get_all(header::HOST).iter().count() != 1
        || headers.get_all(header::ORIGIN).iter().count() > 1
        || headers.get_all("sec-fetch-site").iter().count() > 1
    {
        return false;
    }
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if !allowed_host(host, port, allow_lan_access) {
        return false;
    }
    if headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site") {
        return false;
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        let loopback_origin = (host == format!("127.0.0.1:{port}")
            || host == format!("localhost:{port}"))
            && (origin == format!("http://127.0.0.1:{port}")
                || origin == format!("http://localhost:{port}"));
        if origin != format!("http://{host}") && !loopback_origin {
            return false;
        }
    }
    true
}
fn is_hop_header(name: &str) -> bool {
    matches!(
        name,
        "host"
            | "connection"
            | "keep-alive"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
            | "transfer-encoding"
            | "upgrade"
    )
}
fn clean_headers(headers: &HeaderMap) -> HeaderMap {
    let extra: Vec<String> = headers
        .get_all(header::CONNECTION)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(',').map(|p| p.trim().to_ascii_lowercase()))
        .collect();
    headers
        .iter()
        .filter(|(n, _)| !is_hop_header(n.as_str()) && !extra.iter().any(|e| e == n.as_str()))
        .map(|(n, v)| (n.clone(), v.clone()))
        .collect()
}
pub fn upstream_cookie(existing: Option<&str>, entry_token: &str) -> String {
    let mut cookies: Vec<&str> = existing
        .unwrap_or_default()
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| {
            !matches!(
                s.split('=').next().unwrap_or_default().trim(),
                "entry-token" | "fnos-token" | "fnos-long-token" | "mode"
            )
        })
        .collect();
    let own = format!("entry-token={entry_token}");
    cookies.push(&own);
    cookies.join("; ")
}
pub fn rewrite_location(value: &str, upstream: &url::Url, host: &str) -> String {
    let Ok(target) = upstream.join(value) else {
        return value.to_owned();
    };
    if target.origin() != upstream.origin() {
        return value.to_owned();
    }
    format!(
        "http://{host}{}{}",
        target.path(),
        target.query().map(|q| format!("?{q}")).unwrap_or_default()
    )
}
fn rewrite_cookie(value: &str) -> Option<String> {
    let mut parts = value.split(';');
    let first = parts.next()?;
    if matches!(
        first.split('=').next()?.trim(),
        "entry-token" | "fnos-token" | "fnos-long-token" | "mode"
    ) {
        return None;
    }
    let mut result = vec![first.trim().to_owned()];
    for part in parts {
        let part = part.trim();
        let key = part.split('=').next().unwrap_or_default();
        if key.eq_ignore_ascii_case("domain") || key.eq_ignore_ascii_case("secure") {
            continue;
        }
        if part.eq_ignore_ascii_case("samesite=none") {
            result.push("SameSite=Lax".to_owned());
        } else {
            result.push(part.to_owned());
        }
    }
    Some(result.join("; "))
}
async fn target_headers(
    ctx: &ProxyContext,
    upstream: &url::Url,
    headers: &HeaderMap,
) -> Result<HeaderMap> {
    let session = ctx
        .session
        .read()
        .await
        .clone()
        .ok_or_else(|| error("proxy.sessionUnavailable"))?;
    let token = session.entry_token.read().await;
    let mut outgoing = clean_headers(headers);
    outgoing.insert(
        header::COOKIE,
        HeaderValue::from_str(&upstream_cookie(
            headers.get(header::COOKIE).and_then(|v| v.to_str().ok()),
            &token,
        ))
        .map_err(|_| error("proxy.credentialHeaderInvalid"))?,
    );
    outgoing.remove("sec-fetch-site");
    outgoing.remove("sec-fetch-mode");
    outgoing.remove("sec-fetch-dest");
    outgoing.remove("sec-fetch-user");
    if outgoing.contains_key(header::ORIGIN) {
        outgoing.insert(
            header::ORIGIN,
            HeaderValue::from_str(&upstream.origin().ascii_serialization()).unwrap(),
        );
    }
    if let Some(referer) = headers.get(header::REFERER).and_then(|v| v.to_str().ok()) {
        if let Ok(url) = url::Url::parse(referer) {
            if url.scheme() == "http"
                && url.port_or_known_default() == Some(ctx.local_port)
                && url.host_str().is_some_and(|host| {
                    allowed_host(
                        &format!("{host}:{}", ctx.local_port),
                        ctx.local_port,
                        ctx.allow_lan_access,
                    )
                })
            {
                outgoing.insert(
                    header::REFERER,
                    HeaderValue::from_str(&format!(
                        "{}{}",
                        upstream.origin().ascii_serialization(),
                        url.path()
                    ))
                    .unwrap(),
                );
            } else {
                outgoing.remove(header::REFERER);
            }
        }
    }
    Ok(outgoing)
}
async fn forward(State(ctx): State<ProxyContext>, mut request: Request) -> Response {
    if ctx.cancel.is_cancelled() {
        return fail(
            StatusCode::SERVICE_UNAVAILABLE,
            "proxyStopped",
            "The proxy has stopped.",
        );
    }
    if !is_allowed_request(request.headers(), ctx.local_port, ctx.allow_lan_access) {
        return fail(
            StatusCode::FORBIDDEN,
            "hostNotAllowed",
            "The request address is not allowed; cross-site or unexpected Host requests are blocked.",
        );
    }
    // The validated request authority must survive redirects on remote LAN clients.
    let local_host = request
        .headers()
        .get(header::HOST)
        .unwrap()
        .to_str()
        .unwrap()
        .to_owned();
    // Use one address snapshot for URL, headers and redirects throughout this request.
    let upstream = ctx.upstream.read().unwrap().clone();
    let mut target = upstream.clone();
    target.set_path(request.uri().path());
    target.set_query(request.uri().query());
    let headers = match target_headers(&ctx, &upstream, request.headers()).await {
        Ok(v) => v,
        Err(_) => {
            return fail(
                StatusCode::UNAUTHORIZED,
                "sessionExpired",
                "The NAS session is unavailable. Reconnect in the FN Proxy desktop app.",
            )
        }
    };
    ctx.requests.fetch_add(1, Ordering::Relaxed);
    if request
        .headers()
        .get(header::UPGRADE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.eq_ignore_ascii_case("websocket"))
    {
        let (mut parts, body) = request.into_parts();
        let ws = match WebSocketUpgrade::from_request_parts(&mut parts, &ctx).await {
            Ok(ws) => ws,
            Err(_) => {
                return fail(
                    StatusCode::BAD_REQUEST,
                    "wsHandshakeInvalid",
                    "The WebSocket handshake is invalid.",
                )
            }
        };
        request = Request::from_parts(parts, body);
        target
            .set_scheme(if upstream.scheme() == "https" {
                "wss"
            } else {
                "ws"
            })
            .unwrap();
        let mut remote = match target.as_str().into_client_request() {
            Ok(v) => v,
            Err(_) => {
                return fail(
                    StatusCode::BAD_REQUEST,
                    "wsUrlInvalid",
                    "The WebSocket address is invalid.",
                )
            }
        };
        for (name, value) in headers
            .iter()
            .filter(|(n, _)| !n.as_str().starts_with("sec-websocket-"))
        {
            remote.headers_mut().insert(name.clone(), value.clone());
        }
        if let Some(protocol) = request.headers().get("sec-websocket-protocol") {
            remote
                .headers_mut()
                .insert("sec-websocket-protocol", protocol.clone());
        }
        let connected =
            tokio::time::timeout(std::time::Duration::from_secs(25), connect_async(remote)).await;
        let (socket, response) = match connected {
            Ok(Ok(v)) => v,
            _ => {
                return fail(
                    StatusCode::BAD_GATEWAY,
                    "wsUpstreamFailed",
                    "The upstream WebSocket connection failed.",
                )
            }
        };
        let selected = response
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let ws = if let Some(p) = selected {
            ws.protocols([p])
        } else {
            ws
        };
        return ws.on_upgrade(move |local| bridge(local, socket, ctx.cancel));
    }
    let (parts, body) = request.into_parts();
    let outgoing = ctx
        .http
        .request(parts.method, target)
        .headers(headers)
        .body(reqwest::Body::wrap_stream(body.into_data_stream()));
    let response =
        match tokio::time::timeout(std::time::Duration::from_secs(30), outgoing.send()).await {
            Ok(Ok(r)) => r,
            _ => {
                return fail(
                    StatusCode::BAD_GATEWAY,
                    "upstreamUnavailable",
                    "Connecting to the NAS service failed or timed out.",
                )
            }
        };
    let status = response.status();
    let mut forwarded = clean_headers(response.headers());
    forwarded.remove(header::SET_COOKIE);
    for cookie in response.headers().get_all(header::SET_COOKIE) {
        if let Some(cookie) = cookie.to_str().ok().and_then(rewrite_cookie) {
            if let Ok(value) = HeaderValue::from_str(&cookie) {
                forwarded.append(header::SET_COOKIE, value);
            }
        }
    }
    if let Some(location) = response
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
    {
        if let Ok(value) =
            HeaderValue::from_str(&rewrite_location(location, &upstream, &local_host))
        {
            forwarded.insert(header::LOCATION, value);
        }
    }
    // Stream uploads, downloads and SSE rather than buffering entire responses.
    let stream = response
        .bytes_stream()
        .take_until(ctx.cancel.cancelled_owned());
    let mut outgoing = Response::new(Body::from_stream(stream));
    *outgoing.status_mut() = status;
    *outgoing.headers_mut() = forwarded;
    outgoing
}
async fn bridge(
    local: WebSocket,
    remote: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    cancel: CancellationToken,
) {
    let (mut local_tx, mut local_rx) = local.split();
    let (mut remote_tx, mut remote_rx) = remote.split();
    let to_remote = async {
        while let Some(Ok(m)) = local_rx.next().await {
            let m = match m {
                LocalMessage::Text(t) => RemoteMessage::Text(t.as_str().to_owned().into()),
                LocalMessage::Binary(b) => RemoteMessage::Binary(b),
                LocalMessage::Ping(b) => RemoteMessage::Ping(b),
                LocalMessage::Pong(b) => RemoteMessage::Pong(b),
                LocalMessage::Close(_) => RemoteMessage::Close(None),
            };
            if remote_tx.send(m).await.is_err() {
                break;
            }
        }
    };
    let to_local = async {
        while let Some(Ok(m)) = remote_rx.next().await {
            let m = match m {
                RemoteMessage::Text(t) => LocalMessage::Text(t.as_str().to_owned().into()),
                RemoteMessage::Binary(b) => LocalMessage::Binary(b),
                RemoteMessage::Ping(b) => LocalMessage::Ping(b),
                RemoteMessage::Pong(b) => LocalMessage::Pong(b),
                RemoteMessage::Close(_) => LocalMessage::Close(None),
                RemoteMessage::Frame(_) => continue,
            };
            if local_tx.send(m).await.is_err() {
                break;
            }
        }
    };
    tokio::select! {_=to_remote=>{},_=to_local=>{},_=cancel.cancelled()=>{}}
}
// Reserve only ports that are not already owned by this proxy.
pub fn additional_routes(
    routes: &[ServiceRoute],
    listeners: &[ListenerInfo],
) -> Result<Vec<ServiceRoute>> {
    Ok(routes
        .iter()
        .filter(|route| {
            route.enabled
                && !listeners.iter().any(|listener| {
                    listener.local_url == format!("http://127.0.0.1:{}/", route.local_port)
                })
        })
        .cloned()
        .collect())
}
fn listen_address(allow_lan_access: bool) -> std::net::Ipv4Addr {
    if allow_lan_access {
        std::net::Ipv4Addr::UNSPECIFIED
    } else {
        std::net::Ipv4Addr::LOCALHOST
    }
}
pub async fn start(
    routes: &[ServiceRoute],
    fn_id: &str,
    hub: SessionHub,
    counter: Arc<AtomicU64>,
    allow_lan_access: bool,
) -> Result<Vec<ProxyHandle>> {
    validate_routes(routes, fn_id)?;
    let routes: Vec<_> = routes.iter().filter(|r| r.enabled).collect();
    if routes.is_empty() {
        return Err(error("proxy.noServiceEnabled"));
    }
    // Reserve every port first, so one conflict cannot leave a half-started proxy.
    let mut reserved = Vec::new();
    for route in &routes {
        reserved.push(
            TcpListener::bind((listen_address(allow_lan_access), route.local_port))
                .await
                .map_err(|e| {
                    error_with(
                        "proxy.listenFailed",
                        [
                            ("port", route.local_port.to_string()),
                            ("detail", e.to_string()),
                        ],
                    )
                })?,
        );
    }
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut handles = Vec::new();
    for (route, listener) in routes.into_iter().zip(reserved) {
        let cancel = CancellationToken::new();
        let upstream = Arc::new(StdRwLock::new(validate_upstream(&route.upstream, fn_id)?));
        let ctx = ProxyContext {
            session: hub.clone(),
            upstream: upstream.clone(),
            local_port: route.local_port,
            allow_lan_access,
            requests: counter.clone(),
            cancel: cancel.clone(),
            http: http.clone(),
        };
        let app = Router::new().fallback(any(forward)).with_state(ctx);
        let stop = cancel.clone();
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(stop.cancelled_owned())
                .await;
        });
        handles.push(ProxyHandle {
            route_id: route.id.clone(),
            upstream,
            info: ListenerInfo {
                name: route.name.clone(),
                local_url: format!("http://127.0.0.1:{}/", route.local_port),
                upstream: route.upstream.clone(),
                nas_port: route.nas_port,
            },
            cancel,
            task,
        });
    }
    Ok(handles)
}
// Prepare new listeners before persisting, then reuse existing ports and release removed ones.
// A bind, validation or storage failure leaves the old configuration and traffic intact.
pub async fn apply_routes(
    handles: &mut Vec<ProxyHandle>,
    routes: &[ServiceRoute],
    fn_id: &str,
    hub: SessionHub,
    counter: Arc<AtomicU64>,
    allow_lan_access: bool,
    before_apply: impl FnOnce() -> Result<()>,
) -> Result<()> {
    validate_routes(routes, fn_id)?;
    let targets = routes
        .iter()
        .filter(|route| route.enabled)
        .map(|route| Ok((route, validate_upstream(&route.upstream, fn_id)?)))
        .collect::<Result<Vec<_>>>()?;
    let listeners = handles
        .iter()
        .map(|handle| handle.info.clone())
        .collect::<Vec<_>>();
    let additions = additional_routes(routes, &listeners)?;
    let new_handles = if additions.is_empty() {
        vec![]
    } else {
        start(&additions, fn_id, hub, counter, allow_lan_access).await?
    };
    if let Err(error) = before_apply() {
        stop(new_handles).await;
        return Err(error);
    }
    let mut removed = Vec::new();
    for mut handle in std::mem::take(handles) {
        if let Some((route, upstream)) = targets.iter().find(|(route, _)| {
            handle.info.local_url == format!("http://127.0.0.1:{}/", route.local_port)
        }) {
            *handle.upstream.write().unwrap() = upstream.clone();
            handle.route_id = route.id.clone();
            handle.info.name = route.name.clone();
            handle.info.nas_port = route.nas_port;
            handle.info.upstream = route.upstream.clone();
            handles.push(handle);
        } else {
            removed.push(handle);
        }
    }
    handles.extend(new_handles);
    stop(removed).await;
    Ok(())
}
// Domain refresh changes only the target of future requests, never the listener or open WebSockets.
pub fn update_upstreams(
    handles: &mut [ProxyHandle],
    routes: &[ServiceRoute],
    fn_id: &str,
    before_apply: impl FnOnce() -> Result<()>,
) -> Result<()> {
    // Validate every target before changing any live listener.
    let updates: Vec<_> = handles
        .iter()
        .map(|handle| {
            let route = routes
                .iter()
                .find(|route| {
                    route.id == handle.route_id
                        && route.nas_port == handle.info.nas_port
                        && handle.info.local_url
                            == format!("http://127.0.0.1:{}/", route.local_port)
                })
                .ok_or_else(|| error("proxy.routesChangedDuringSync"))?;
            Ok((
                route.upstream.clone(),
                validate_upstream(&route.upstream, fn_id)?,
            ))
        })
        .collect::<Result<_>>()?;
    before_apply()?;
    for (handle, (address, url)) in handles.iter_mut().zip(updates) {
        *handle.upstream.write().unwrap() = url;
        handle.info.upstream = address;
    }
    Ok(())
}
pub async fn stop(handles: Vec<ProxyHandle>) {
    for handle in &handles {
        handle.cancel.cancel();
    }
    for mut handle in handles {
        if tokio::time::timeout(std::time::Duration::from_secs(5), &mut handle.task)
            .await
            .is_err()
        {
            handle.task.abort();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn no_untrusted_cookie_can_replace_entry_token() {
        assert_eq!(
            upstream_cookie(
                Some("app=ok; entry-token=attacker; fnos-token=private"),
                "owned"
            ),
            "app=ok; entry-token=owned"
        );
    }
    #[test]
    fn blocks_dns_rebinding_and_browser_cross_site_requests() {
        let mut h = HeaderMap::new();
        h.insert(header::HOST, "127.0.0.1:18084".parse().unwrap());
        assert!(is_allowed_request(&h, 18084, false));
        h.insert(header::ORIGIN, "https://evil.test".parse().unwrap());
        assert!(!is_allowed_request(&h, 18084, false));
        h.remove(header::ORIGIN);
        h.insert(header::HOST, "evil.test:18084".parse().unwrap());
        assert!(!is_allowed_request(&h, 18084, false));
    }
    #[test]
    fn duplicate_security_headers_are_rejected() {
        for name in [
            header::HOST,
            header::ORIGIN,
            axum::http::HeaderName::from_static("sec-fetch-site"),
        ] {
            let mut headers = HeaderMap::new();
            headers.insert(header::HOST, "192.168.1.10:18084".parse().unwrap());
            if name != header::HOST {
                headers.insert(
                    name.clone(),
                    if name == header::ORIGIN {
                        "http://192.168.1.10:18084"
                    } else {
                        "same-origin"
                    }
                    .parse()
                    .unwrap(),
                );
            }
            headers.append(name, "evil.test".parse().unwrap());
            assert!(!is_allowed_request(&headers, 18084, true));
        }
    }
    #[test]
    fn only_same_origin_redirects_are_rewritten() {
        let u = url::Url::parse("https://hash.my-nas.fnos.net/").unwrap();
        assert_eq!(
            rewrite_location("/login?next=a", &u, "127.0.0.1:18084"),
            "http://127.0.0.1:18084/login?next=a"
        );
        assert_eq!(
            rewrite_location("https://external.test/", &u, "127.0.0.1:18084"),
            "https://external.test/"
        );
    }
    #[test]
    fn gateway_cookies_never_reach_local_clients() {
        assert!(rewrite_cookie("entry-token=private; Path=/").is_none());
        assert_eq!(
            rewrite_cookie("app=x; Domain=.my-nas.fnos.net; Secure; HttpOnly; SameSite=None")
                .unwrap(),
            "app=x; HttpOnly; SameSite=Lax"
        );
    }
    #[test]
    fn lan_policy_keeps_loopback_default_and_rejects_rebinding_and_cross_site_origins() {
        assert_eq!(listen_address(false), std::net::Ipv4Addr::LOCALHOST);
        assert_eq!(listen_address(true), std::net::Ipv4Addr::UNSPECIFIED);
        let mut headers = HeaderMap::new();
        for host in [
            "192.168.1.10:18084",
            "10.0.0.5:18084",
            "172.16.0.5:18084",
            "169.254.1.2:18084",
        ] {
            headers.insert(header::HOST, host.parse().unwrap());
            assert!(!is_allowed_request(&headers, 18084, false));
            assert!(is_allowed_request(&headers, 18084, true));
            headers.insert(header::ORIGIN, format!("http://{host}").parse().unwrap());
            assert!(is_allowed_request(&headers, 18084, true));
            headers.insert(header::ORIGIN, "http://192.168.1.11:18084".parse().unwrap());
            assert!(!is_allowed_request(&headers, 18084, true));
            headers.remove(header::ORIGIN);
        }
        for host in [
            "evil.test:18084",
            "192.168.1.10:18085",
            "8.8.8.8:18084",
            "0.0.0.0:18084",
            "172.32.0.1:18084",
            "192.168.1.10",
            "192.168.1.10:018084",
        ] {
            headers.insert(header::HOST, host.parse().unwrap());
            assert!(!is_allowed_request(&headers, 18084, true));
        }
        headers.insert(header::HOST, "192.168.1.10:18084".parse().unwrap());
        headers.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert!(!is_allowed_request(&headers, 18084, true));
        assert!(!is_allowed_request(&HeaderMap::new(), 18084, true));
    }
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod integration_tests;
