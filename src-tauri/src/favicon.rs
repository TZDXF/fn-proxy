use crate::types::validate_upstream;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::StreamExt;
use quick_xml::{events::Event, name::ResolveResult, NsReader};
use scraper::{Html, Selector};
use std::{collections::HashSet, time::Duration};
use tokio::sync::Semaphore;

const MAX_ICON_BYTES: usize = 256 * 1024;
const MAX_HTML_BYTES: usize = 512 * 1024;
const MAX_ICON_CANDIDATES: usize = 8;
const MAX_REDIRECTS: usize = 3;
static REQUESTS: Semaphore = Semaphore::const_new(4);

pub async fn fetch(upstream: &str, fn_id: &str, token: &str) -> Option<String> {
    // Validate the authenticated service origin before following any HTML-provided URLs.
    let root = validate_upstream(upstream, fn_id).ok()?;
    tokio::time::timeout(Duration::from_secs(12), async {
        let _permit = REQUESTS.acquire().await.ok()?;
        let client = http_client()?;
        discover_icon(&client, &root, token).await
    })
    .await
    .ok()
    .flatten()
}

fn http_client() -> Option<reqwest::Client> {
    reqwest::Client::builder()
        // Redirects are handled manually, with an origin check before every request.
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(5))
        .build()
        .ok()
}

async fn discover_icon(client: &reqwest::Client, root: &url::Url, token: &str) -> Option<String> {
    let favicon = root.join("/favicon.ico").ok()?;
    if let Some(icon) = fetch_icon(client, root, &favicon, token).await {
        return Some(icon);
    }
    let (page_url, html) = fetch_bytes(client, root, root, token, MAX_HTML_BYTES).await?;
    // Parsing happens in a synchronous helper; no DOM is retained across network awaits.
    for url in icon_urls(&String::from_utf8_lossy(&html), &page_url, root) {
        if url == favicon {
            continue;
        }
        if let Some(icon) = fetch_icon(client, root, &url, token).await {
            return Some(icon);
        }
    }
    None
}

async fn fetch_icon(
    client: &reqwest::Client,
    root: &url::Url,
    url: &url::Url,
    token: &str,
) -> Option<String> {
    let (_, bytes) = fetch_bytes(client, root, url, token, MAX_ICON_BYTES).await?;
    let mime = image_mime(&bytes)?;
    Some(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn same_origin(url: &url::Url, root: &url::Url) -> bool {
    url.origin() == root.origin() && url.username().is_empty() && url.password().is_none()
}

async fn fetch_bytes(
    client: &reqwest::Client,
    root: &url::Url,
    url: &url::Url,
    token: &str,
    limit: usize,
) -> Option<(url::Url, Vec<u8>)> {
    let mut url = url.clone();
    for redirects in 0..=MAX_REDIRECTS {
        if !same_origin(&url, root) {
            return None;
        }
        url.set_fragment(None);
        let response = client
            .get(url.clone())
            .header("Cookie", format!("entry-token={token}"))
            .send()
            .await
            .ok()?;
        if response.status().is_redirection() {
            if redirects == MAX_REDIRECTS {
                return None;
            }
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)?
                .to_str()
                .ok()?;
            url = url.join(location).ok()?;
            continue;
        }
        if !response.status().is_success()
            || response
                .content_length()
                .is_some_and(|len| len > limit as u64)
        {
            return None;
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.ok()?;
            if chunk.len() > limit - bytes.len() {
                return None;
            }
            bytes.extend_from_slice(&chunk);
        }
        return Some((url, bytes));
    }
    None
}

fn icon_urls(html: &str, page_url: &url::Url, root: &url::Url) -> Vec<url::Url> {
    let document = Html::parse_document(html);
    let base_selector = Selector::parse("base[href]").expect("valid base selector");
    let base = document
        .select(&base_selector)
        .next()
        .and_then(|element| page_url.join(element.value().attr("href")?.trim()).ok())
        .unwrap_or_else(|| page_url.clone());
    let link_selector = Selector::parse("link[rel][href]").expect("valid icon selector");
    let mut icons = Vec::new();
    let mut touch_icons = Vec::new();
    let mut seen = HashSet::new();
    for link in document.select(&link_selector) {
        let rel = link.value().attr("rel").unwrap_or_default();
        let target = if rel
            .split_ascii_whitespace()
            .any(|rel| rel.eq_ignore_ascii_case("icon"))
        {
            &mut icons
        } else if rel.split_ascii_whitespace().any(|rel| {
            rel.eq_ignore_ascii_case("apple-touch-icon")
                || rel.eq_ignore_ascii_case("apple-touch-icon-precomposed")
        }) {
            &mut touch_icons
        } else {
            continue;
        };
        if target.len() >= MAX_ICON_CANDIDATES {
            continue;
        }
        let href = link.value().attr("href").unwrap_or_default().trim();
        if href.is_empty() {
            continue;
        }
        let Ok(mut url) = base.join(href) else {
            continue;
        };
        if !same_origin(&url, root) {
            continue;
        }
        url.set_fragment(None);
        if seen.insert(url.clone()) {
            target.push(url);
        }
    }
    icons
        .into_iter()
        .chain(touch_icons)
        .take(MAX_ICON_CANDIDATES)
        .collect()
}

fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    // Inspect the data rather than trusting a server's Content-Type. SVG is returned only
    // as an image data URL for <img>, never as inline markup or a navigable document.
    if bytes.starts_with(&[0, 0, 1, 0]) && bytes.len() >= 6 {
        Some("image/x-icon")
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        Some("image/webp")
    } else if is_svg_image(bytes) {
        Some("image/svg+xml")
    } else {
        None
    }
}

fn is_svg_image(bytes: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return false;
    };
    let mut reader = NsReader::from_str(text.trim_start_matches('\u{feff}'));
    let mut depth = 0;
    let mut root_seen = false;
    loop {
        let Ok((namespace, event)) = reader.read_resolved_event() else {
            return false;
        };
        match event {
            Event::Start(ref tag) | Event::Empty(ref tag) => {
                if depth == 0 {
                    if root_seen
                        || tag.local_name().as_ref() != "svg"
                        || !matches!(namespace, ResolveResult::Bound(ns) if ns.as_ref() == "http://www.w3.org/2000/svg")
                    {
                        return false;
                    }
                    root_seen = true;
                }
                if matches!(event, Event::Start(_)) {
                    depth += 1;
                }
            }
            Event::End(_) => {
                if depth == 0 {
                    return false;
                }
                depth -= 1;
            }
            Event::Text(text)
                if depth == 0 && !text.as_ref().as_bytes().iter().all(u8::is_ascii_whitespace) =>
            {
                return false;
            }
            // No DTD/entity definitions or non-XML content outside the root element.
            Event::DocType(_) => return false,
            Event::CData(_) | Event::GeneralRef(_) if depth == 0 => return false,
            Event::Eof => return root_seen && depth == 0,
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "favicon_tests.rs"]
mod tests;
