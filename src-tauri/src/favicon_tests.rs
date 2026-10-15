use super::*;
use axum::{
    body::{Body, Bytes},
    http::{header, HeaderMap, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tokio::net::TcpListener;

const ICON: &[u8] = &[0, 0, 1, 0, 1, 0];

async fn server(router: Router) -> (url::Url, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = url::Url::parse(&format!("http://{}/", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (url, task)
}

async fn fetch_url(root: &url::Url, path: &str) -> Option<String> {
    fetch_icon(
        &http_client().unwrap(),
        root,
        &root.join(path).unwrap(),
        "fixture",
    )
    .await
}

async fn discover(root: &url::Url) -> Option<String> {
    discover_icon(&http_client().unwrap(), root, "fixture").await
}

fn expected_icon() -> String {
    format!("data:image/x-icon;base64,{}", STANDARD.encode(ICON))
}

#[tokio::test]
async fn fetches_authenticated_default_icon_without_fetching_html() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let router = Router::new()
        .route(
            "/favicon.ico",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::COOKIE], "entry-token=fixture");
                ICON
            }),
        )
        .route(
            "/",
            get(move || async move {
                observed.fetch_add(1, Ordering::Relaxed);
                "unexpected"
            }),
        );
    let (root, task) = server(router).await;
    assert_eq!(discover(&root).await, Some(expected_icon()));
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    task.abort();
}

#[tokio::test]
async fn falls_back_to_html_with_redirect_base_relative_url_and_entity_decoding() {
    let router = Router::new()
        .route("/", get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/ui/login.html")]) }))
        .route("/ui/login.html", get(|headers: HeaderMap| async move {
            assert_eq!(headers[header::COOKIE], "entry-token=fixture");
            r#"<base href='../assets/'><LINK HREF='brand.ico?v=1&amp;size=32' REL='SHORTCUT ICON'>"#
        }))
        .route("/assets/brand.ico", get(|headers: HeaderMap, uri: axum::http::Uri| async move {
            assert_eq!(headers[header::COOKIE], "entry-token=fixture");
            assert_eq!(uri.query(), Some("v=1&size=32"));
            ICON
        }));
    let (root, task) = server(router).await;
    assert_eq!(discover(&root).await, Some(expected_icon()));
    task.abort();
}

#[tokio::test]
async fn resolves_relative_icons_against_the_final_page_url_without_a_base() {
    let router = Router::new()
        .route(
            "/",
            get(|| async {
                (
                    StatusCode::TEMPORARY_REDIRECT,
                    [(header::LOCATION, "/web/index.html")],
                )
            }),
        )
        .route(
            "/web/index.html",
            get(|| async { "<link rel=icon href=images/logo.ico>" }),
        )
        .route("/web/images/logo.ico", get(|| async { ICON }));
    let (root, task) = server(router).await;
    assert_eq!(discover(&root).await, Some(expected_icon()));
    task.abort();
}

#[tokio::test]
async fn tries_remaining_candidates_and_touch_icons_after_missing_or_invalid_icons() {
    let router = Router::new()
        .route(
            "/",
            get(|| async {
                r#"
            <link rel=apple-touch-icon href=/touch.ico>
            <link rel=icon href=/missing.ico>
            <link rel=icon href=/not-an-image.ico>
        "#
            }),
        )
        .route(
            "/not-an-image.ico",
            get(|| async { "<html>app login page</html>" }),
        )
        .route("/touch.ico", get(|| async { ICON }));
    let (root, task) = server(router).await;
    assert_eq!(discover(&root).await, Some(expected_icon()));
    task.abort();
}

#[tokio::test]
async fn follows_same_origin_icon_redirects() {
    let router = Router::new()
        .route(
            "/favicon.ico",
            get(|| async { (StatusCode::FOUND, [(header::LOCATION, "/assets/icon.ico")]) }),
        )
        .route(
            "/assets/icon.ico",
            get(|headers: HeaderMap| async move {
                assert_eq!(headers[header::COOKIE], "entry-token=fixture");
                ICON
            }),
        );
    let (root, task) = server(router).await;
    assert_eq!(
        fetch_url(&root, "/favicon.ico").await,
        Some(expected_icon())
    );
    task.abort();
}

#[tokio::test]
async fn never_requests_cross_origin_html_icons_or_redirect_targets() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let (outside, outside_task) = server(Router::new().fallback(get(move || async move {
        observed.fetch_add(1, Ordering::Relaxed);
        ICON
    })))
    .await;
    let icon_url = outside.join("/secret.ico").unwrap().to_string();
    let html = format!("<link rel=icon href='{icon_url}'><link rel=icon href=/redirect.ico>");
    let router = Router::new()
        .route("/", get(move || async move { html }))
        .route(
            "/favicon.ico",
            get({
                let target = icon_url.clone();
                move || async move { (StatusCode::FOUND, [(header::LOCATION, target)]) }
            }),
        )
        .route(
            "/redirect.ico",
            get(move || async move { (StatusCode::FOUND, [(header::LOCATION, icon_url)]) }),
        );
    let (root, task) = server(router).await;
    assert!(discover(&root).await.is_none());
    assert_eq!(hits.load(Ordering::Relaxed), 0);
    task.abort();
    outside_task.abort();
}

#[tokio::test]
async fn bounds_redirect_loops() {
    let hits = Arc::new(AtomicUsize::new(0));
    let observed = hits.clone();
    let router = Router::new().route(
        "/favicon.ico",
        get(move || async move {
            observed.fetch_add(1, Ordering::Relaxed);
            (StatusCode::FOUND, [(header::LOCATION, "/favicon.ico")])
        }),
    );
    let (root, task) = server(router).await;
    assert!(fetch_url(&root, "/favicon.ico").await.is_none());
    assert_eq!(hits.load(Ordering::Relaxed), MAX_REDIRECTS + 1);
    task.abort();
}

#[tokio::test]
async fn missing_and_invalid_html_keep_the_default_icon() {
    for (status, body) in [
        (StatusCode::OK, "<html>Login</html>"),
        (StatusCode::NOT_FOUND, "missing"),
        (StatusCode::OK, "<link rel=stylesheet href=/style.css>"),
    ] {
        let (root, task) =
            server(Router::new().route("/", get(move || async move { (status, body) }))).await;
        assert!(discover(&root).await.is_none());
        task.abort();
    }
}

#[tokio::test]
async fn rejects_oversized_fixed_and_streamed_images_and_html() {
    for (path, limit) in [("/favicon.ico", MAX_ICON_BYTES), ("/", MAX_HTML_BYTES)] {
        for streamed in [false, true] {
            let router = Router::new().route(
                path,
                get(move || async move {
                    if streamed {
                        let stream = futures_util::stream::iter([
                            Ok::<_, std::io::Error>(Bytes::from(vec![0; limit])),
                            Ok(Bytes::from_static(&[1])),
                        ]);
                        Body::from_stream(stream).into_response()
                    } else {
                        vec![0; limit + 1].into_response()
                    }
                }),
            );
            let (root, task) = server(router).await;
            let client = http_client().unwrap();
            assert!(
                fetch_bytes(&client, &root, &root.join(path).unwrap(), "fixture", limit)
                    .await
                    .is_none()
            );
            task.abort();
        }
    }
}

#[tokio::test]
async fn slow_pages_fail_within_the_request_timeout() {
    let router = Router::new().route(
        "/",
        get(|| async {
            tokio::time::sleep(Duration::from_secs(1)).await;
            "<link rel=icon href=/logo.ico>"
        }),
    );
    let (root, task) = server(router).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_millis(30))
        .build()
        .unwrap();
    assert!(
        fetch_bytes(&client, &root, &root, "fixture", MAX_HTML_BYTES)
            .await
            .is_none()
    );
    task.abort();
}

#[tokio::test]
async fn rejects_untrusted_upstreams_before_requesting() {
    for upstream in [
        "http://127.0.0.1/",
        "https://other.fnos.net/",
        "https://my-nas.fnos.net.evil.test/",
        "https://my-nas.fnos.net/path",
    ] {
        assert!(fetch(upstream, "my-nas", "fixture").await.is_none());
    }
}

#[test]
fn parses_html_links_with_case_insensitive_rel_tokens_and_varied_attribute_syntax() {
    let root = url::Url::parse("https://app.my-nas.fnos.net/").unwrap();
    let html = r#"
        <!-- <link rel=icon href=/comment.ico> -->
        <script>const example = '<link rel=icon href=/script.ico>';</script>
        <link rel=not-icon href=/not-icon.ico>
        <link rel=stylesheet href=/style.css>
        <LINK HREF='/assets/logo.png?v=1&amp;size=32' REL='shortcut ICON'>
        <link rel="icon alternate" href=icons/unquoted.ico>
        <link rel=ICON href="//app.my-nas.fnos.net/root.ico#ignored">
        <link href="https://app.my-nas.fnos.net/absolute.ico" rel="icon">
    "#;
    let urls = icon_urls(html, &root, &root);
    let paths: Vec<_> = urls.iter().map(|url| url.path()).collect();
    assert_eq!(
        paths,
        [
            "/assets/logo.png",
            "/icons/unquoted.ico",
            "/root.ico",
            "/absolute.ico"
        ]
    );
    assert_eq!(urls[0].query(), Some("v=1&size=32"));
    assert_eq!(urls[2].fragment(), None);
}

#[test]
fn rejects_untrusted_icon_urls_and_respects_an_external_base() {
    let root = url::Url::parse("https://app.my-nas.fnos.net/").unwrap();
    let html = r#"
        <link rel=icon href="https://evil.test/icon.ico">
        <link rel=icon href="http://app.my-nas.fnos.net/icon.ico">
        <link rel=icon href="https://app.my-nas.fnos.net:8443/icon.ico">
        <link rel=icon href="https://user:password@app.my-nas.fnos.net/icon.ico">
        <link rel=icon href="https://other.my-nas.fnos.net/icon.ico">
        <link rel=icon href="data:image/png;base64,AAAA">
        <link rel=icon href="javascript:alert(1)">
        <link rel=icon href="file:///icon.ico">
        <link rel=icon href="">
    "#;
    assert!(icon_urls(html, &root, &root).is_empty());
    assert!(icon_urls(
        "<base href='https://evil.test/'><link rel=icon href=logo.ico>",
        &root,
        &root
    )
    .is_empty());
}

#[test]
fn prioritizes_standard_icons_and_deduplicates_and_limits_candidates() {
    let root = url::Url::parse("https://app.my-nas.fnos.net/").unwrap();
    let html = "<link rel=apple-touch-icon href=/touch.png><link rel=icon href=/logo.ico><link rel='shortcut icon' href=/logo.ico#same><link rel=apple-touch-icon-precomposed href=/touch2.png>";
    let urls = icon_urls(html, &root, &root);
    let paths: Vec<_> = urls.iter().map(|url| url.path()).collect();
    assert_eq!(paths, ["/logo.ico", "/touch.png", "/touch2.png"]);
    let html = (0..100)
        .map(|i| format!("<link rel=icon href=/{i}.ico>"))
        .collect::<String>();
    assert_eq!(icon_urls(&html, &root, &root).len(), MAX_ICON_CANDIDATES);
}

#[test]
fn recognizes_common_raster_icons_but_not_html_or_unnamespaced_xml() {
    for bytes in [
        b"\x89PNG\r\n\x1a\n".as_slice(),
        b"\xff\xd8\xff",
        b"GIF89a",
        b"GIF87a",
        b"RIFF0000WEBP",
    ] {
        assert!(image_mime(bytes).is_some());
    }
    for bytes in [
        b"".as_slice(),
        b"<svg></svg>",
        b"<html></html>",
        &[0, 0, 1, 0],
    ] {
        assert!(image_mime(bytes).is_none());
    }
}

#[tokio::test]
async fn discovers_svg_icon_from_aether_style_html_when_favicon_is_missing() {
    const SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><style>@media (prefers-color-scheme: dark) { path { fill: white; } }</style><path fill="black" d="M0 0h32v32H0z"/></svg>"#;
    let router = Router::new()
        .route("/", get(|| async { r#"<!DOCTYPE html><html><head><link rel="icon" type="image/svg+xml" href="/aether_adaptive.svg"></head></html>"# }))
        .route("/aether_adaptive.svg", get(|| async { ([(header::CONTENT_TYPE, "image/svg+xml")], SVG) }));
    let (root, task) = server(router).await;
    assert_eq!(
        discover(&root).await,
        Some(format!(
            "data:image/svg+xml;base64,{}",
            STANDARD.encode(SVG)
        ))
    );
    task.abort();
}

#[test]
fn accepts_svg_with_namespaces_xml_declarations_comments_and_utf8_bom() {
    for svg in [
        r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#,
        r#"<?xml version="1.0"?><!-- icon --><svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#,
        "\u{feff}<s:svg xmlns:s='http://www.w3.org/2000/svg'><s:g/></s:svg>",
    ] {
        assert_eq!(image_mime(svg.as_bytes()), Some("image/svg+xml"));
    }
}

#[test]
fn rejects_html_malformed_svg_and_dtds_including_external_entities() {
    for svg in [
        "<html><svg xmlns='http://www.w3.org/2000/svg'/></html>",
        "<svg xmlns='https://evil.test/'/>",
        "<svg xmlns='http://www.w3.org/2000/svg'><path></svg>",
        "<svg xmlns='http://www.w3.org/2000/svg'>",
        "<svg xmlns='http://www.w3.org/2000/svg'/><svg xmlns='http://www.w3.org/2000/svg'/>",
        "<!DOCTYPE svg SYSTEM 'https://evil.test/remote.dtd'><svg xmlns='http://www.w3.org/2000/svg'/>",
        "<!DOCTYPE svg [<!ENTITY secret SYSTEM 'file:///private'>]><svg xmlns='http://www.w3.org/2000/svg'>&secret;</svg>",
        "not XML<svg xmlns='http://www.w3.org/2000/svg'/>",
    ] {
        assert!(image_mime(svg.as_bytes()).is_none());
    }
}
