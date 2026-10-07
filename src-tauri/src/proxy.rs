use crate::{
    auth::NasSession,
    error::{error, Result},
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
fn fail(status: StatusCode, message: &str) -> Response {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "application/json; charset=utf-8")
        .body(Body::from(serde_json::json!({"error":message}).to_string()))
        .unwrap()
}
pub fn is_local_request(headers: &HeaderMap, port: u16) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return false;
    }
    if headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) == Some("cross-site") {
        return false;
    }
    if let Some(origin) = headers.get(header::ORIGIN) {
        let Ok(origin) = origin.to_str() else {
            return false;
        };
        if origin != format!("http://127.0.0.1:{port}")
            && origin != format!("http://localhost:{port}")
        {
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
pub fn rewrite_location(value: &str, upstream: &url::Url, port: u16) -> String {
    let Ok(target) = upstream.join(value) else {
        return value.to_owned();
    };
    if target.origin() != upstream.origin() {
        return value.to_owned();
    }
    format!(
        "http://127.0.0.1:{port}{}{}",
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
        .ok_or_else(|| error("NAS 尚未登录或认证连接已失效"))?;
    let token = session.entry_token.read().await;
    let mut outgoing = clean_headers(headers);
    outgoing.insert(
        header::COOKIE,
        HeaderValue::from_str(&upstream_cookie(
            headers.get(header::COOKIE).and_then(|v| v.to_str().ok()),
            &token,
        ))
        .map_err(|_| error("服务访问凭据无效"))?,
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
            if matches!(url.host_str(), Some("localhost" | "127.0.0.1")) {
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
        return fail(StatusCode::SERVICE_UNAVAILABLE, "代理已停止");
    }
    if !is_local_request(request.headers(), ctx.local_port) {
        return fail(
            StatusCode::FORBIDDEN,
            "仅允许本机访问；已阻止跨站或异常 Host 请求",
        );
    }
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
                "NAS 会话不可用，请在桌面应用中重新连接",
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
            Err(_) => return fail(StatusCode::BAD_REQUEST, "WebSocket 握手无效"),
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
            Err(_) => return fail(StatusCode::BAD_REQUEST, "WebSocket 地址无效"),
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
            _ => return fail(StatusCode::BAD_GATEWAY, "上游 WebSocket 连接失败"),
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
            _ => return fail(StatusCode::BAD_GATEWAY, "连接 NAS 服务失败或等待响应超时"),
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
            HeaderValue::from_str(&rewrite_location(location, &upstream, ctx.local_port))
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
// Keep existing listeners untouched when adding routes to a running proxy.
pub fn additional_routes(
    routes: &[ServiceRoute],
    listeners: &[ListenerInfo],
) -> Result<Vec<ServiceRoute>> {
    for listener in listeners {
        if !routes.iter().any(|route| {
            route.enabled
                && listener.local_url == format!("http://127.0.0.1:{}/", route.local_port)
                && listener.upstream == route.upstream
                && listener.nas_port == route.nas_port
                && listener.name == route.name
        }) {
            return Err(error("请先停止代理再编辑或删除已有映射"));
        }
    }
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
pub async fn start(
    routes: &[ServiceRoute],
    fn_id: &str,
    hub: SessionHub,
    counter: Arc<AtomicU64>,
) -> Result<Vec<ProxyHandle>> {
    validate_routes(routes, fn_id)?;
    let routes: Vec<_> = routes.iter().filter(|r| r.enabled).collect();
    if routes.is_empty() {
        return Err(error("至少启用一个服务映射"));
    }
    // Reserve every port first, so one conflict cannot leave a half-started proxy.
    let mut reserved = Vec::new();
    for route in &routes {
        reserved.push(
            TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, route.local_port))
                .await
                .map_err(|e| {
                    error(format!(
                        "无法监听本地端口 {}：{e}；请选择其他端口",
                        route.local_port
                    ))
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
                .ok_or_else(|| error("域名同步时服务映射已变化，拒绝更新运行中的代理"))?;
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
        assert!(is_local_request(&h, 18084));
        h.insert(header::ORIGIN, "https://evil.test".parse().unwrap());
        assert!(!is_local_request(&h, 18084));
        h.remove(header::ORIGIN);
        h.insert(header::HOST, "evil.test:18084".parse().unwrap());
        assert!(!is_local_request(&h, 18084));
    }
    #[test]
    fn only_same_origin_redirects_are_rewritten() {
        let u = url::Url::parse("https://hash.my-nas.fnos.net/").unwrap();
        assert_eq!(
            rewrite_location("/login?next=a", &u, 18084),
            "http://127.0.0.1:18084/login?next=a"
        );
        assert_eq!(
            rewrite_location("https://external.test/", &u, 18084),
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
}

#[cfg(test)]
#[path = "proxy_tests.rs"]
mod integration_tests;
