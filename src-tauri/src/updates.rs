use std::{cmp::Ordering, time::Duration};

use reqwest::{Client, StatusCode};
use semver::Version;
use serde::{Deserialize, Serialize};

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/TZDXF/fn-proxy/releases/latest";
const RELEASES_URL: &str = "https://github.com/TZDXF/fn-proxy/releases";

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current_version: String,
    latest_version: Option<String>,
    update_available: bool,
    release_url: String,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum UpdateError {
    Network,
    RateLimited,
    InvalidResponse,
    UnexpectedResponse,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn release_info(current: &str, release: Option<Release>) -> Result<UpdateInfo, UpdateError> {
    let mut info = UpdateInfo {
        current_version: current.to_owned(),
        latest_version: None,
        update_available: false,
        release_url: RELEASES_URL.to_owned(),
    };
    if let Some(release) = release {
        let version = Version::parse(
            release
                .tag_name
                .strip_prefix('v')
                .unwrap_or(&release.tag_name),
        )
        .map_err(|_| UpdateError::InvalidResponse)?;
        let current = Version::parse(current).map_err(|_| UpdateError::InvalidResponse)?;
        if release.draft || release.prerelease || !version.pre.is_empty() {
            return Err(UpdateError::InvalidResponse);
        }
        info.update_available = version.cmp_precedence(&current) == Ordering::Greater;
        info.latest_version = Some(version.to_string());
        // Construct a repository-scoped URL; never open a URL supplied by the API response.
        let mut url = url::Url::parse(RELEASES_URL).map_err(|_| UpdateError::InvalidResponse)?;
        url.path_segments_mut()
            .map_err(|_| UpdateError::InvalidResponse)?
            .push("tag")
            .push(&release.tag_name);
        info.release_url = url.to_string();
    }
    Ok(info)
}

async fn fetch_latest(
    client: &Client,
    endpoint: &str,
    current: &str,
) -> Result<UpdateInfo, UpdateError> {
    let response = client
        .get(endpoint)
        .send()
        .await
        .map_err(|_| UpdateError::Network)?;
    match response.status() {
        StatusCode::NOT_FOUND => release_info(current, None),
        StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => Err(UpdateError::RateLimited),
        StatusCode::OK => {
            let release = response
                .json()
                .await
                .map_err(|_| UpdateError::InvalidResponse)?;
            release_info(current, Some(release))
        }
        _ => Err(UpdateError::UnexpectedResponse),
    }
}

#[tauri::command]
pub async fn check_for_updates(app: tauri::AppHandle) -> Result<UpdateInfo, UpdateError> {
    let current = app.package_info().version.to_string();
    // A fresh client has no NAS session, cookie store, or authorization headers.
    let client = Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .user_agent(format!("FN-Proxy/{current}"))
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::ACCEPT,
                "application/vnd.github+json".parse().unwrap(),
            );
            headers.insert("X-GitHub-Api-Version", "2022-11-28".parse().unwrap());
            headers
        })
        .build()
        .map_err(|_| UpdateError::Network)?;
    fetch_latest(&client, LATEST_RELEASE_API, &current).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::get, Router};

    fn release(tag: &str) -> Release {
        Release {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
        }
    }

    #[test]
    fn compares_semantic_versions_and_ignores_build_metadata() {
        assert!(
            release_info("0.9.0", Some(release("v0.10.0")))
                .unwrap()
                .update_available
        );
        assert!(
            !release_info("0.10.0", Some(release("v0.9.0")))
                .unwrap()
                .update_available
        );
        assert!(
            !release_info("0.1.0", Some(release("v0.1.0")))
                .unwrap()
                .update_available
        );
        assert!(
            !release_info("0.1.0+local", Some(release("v0.1.0+build")))
                .unwrap()
                .update_available
        );
        assert!(
            release_info("0.1.0-beta.1", Some(release("v0.1.0")))
                .unwrap()
                .update_available
        );
    }

    #[test]
    fn returns_repository_scoped_urls_and_handles_no_releases() {
        let info = release_info("0.1.0", Some(release("v0.2.0"))).unwrap();
        assert_eq!(info.release_url, format!("{RELEASES_URL}/tag/v0.2.0"));
        assert_eq!(info.latest_version.as_deref(), Some("0.2.0"));
        let empty = release_info("0.1.0", None).unwrap();
        assert_eq!(empty.latest_version, None);
        assert!(!empty.update_available);
        assert_eq!(empty.release_url, RELEASES_URL);
    }

    #[test]
    fn rejects_invalid_tags_and_non_stable_releases() {
        for tag in ["latest", "../../evil", "v0.2.0-beta.1"] {
            assert_eq!(
                release_info("0.1.0", Some(release(tag))),
                Err(UpdateError::InvalidResponse)
            );
        }
        for (draft, prerelease) in [(true, false), (false, true)] {
            let mut value = release("v0.2.0");
            value.draft = draft;
            value.prerelease = prerelease;
            assert_eq!(
                release_info("0.1.0", Some(value)),
                Err(UpdateError::InvalidResponse)
            );
        }
    }

    #[tokio::test]
    async fn handles_github_http_responses_without_real_network() {
        for (status, body, expected) in [
            (StatusCode::NOT_FOUND, "", None),
            (StatusCode::FORBIDDEN, "", Some(UpdateError::RateLimited)),
            (
                StatusCode::TOO_MANY_REQUESTS,
                "",
                Some(UpdateError::RateLimited),
            ),
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                "",
                Some(UpdateError::UnexpectedResponse),
            ),
            (StatusCode::FOUND, "", Some(UpdateError::UnexpectedResponse)),
            (
                StatusCode::OK,
                "invalid JSON",
                Some(UpdateError::InvalidResponse),
            ),
            (
                StatusCode::OK,
                r#"{"tag_name":"v0.2.0","draft":false,"prerelease":false}"#,
                None,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let router = Router::new().route("/latest", get(move || async move { (status, body) }));
            let task = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            let client = Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .unwrap();
            let result = fetch_latest(&client, &format!("http://{address}/latest"), "0.1.0").await;
            task.abort();
            if let Some(error) = expected {
                assert_eq!(result, Err(error));
            } else {
                assert_eq!(result.unwrap().update_available, status == StatusCode::OK);
            }
        }
    }

    #[tokio::test]
    async fn reports_network_timeout() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/latest", listener.local_addr().unwrap());
        let client = Client::builder()
            .timeout(Duration::from_millis(50))
            .build()
            .unwrap();
        assert_eq!(
            fetch_latest(&client, &endpoint, "0.1.0").await,
            Err(UpdateError::Network)
        );
    }
}
