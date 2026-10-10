use crate::text::Text;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Code(Text),
    #[error("{stage}: {source}")]
    Stage {
        stage: &'static str,
        source: Box<AppError>,
    },
    #[error("automatic recovery exhausted after {attempts} attempts")]
    RecoveryExhausted {
        attempts: usize,
        #[source]
        source: Box<AppError>,
    },
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),
    #[error("websocket: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("timeout: {0}")]
    Timeout(#[from] tokio::time::error::Elapsed),
}
impl AppError {
    pub fn at(self, stage: &'static str) -> Self {
        Self::Stage {
            stage,
            source: Box::new(self),
        }
    }
    pub fn stage(&self) -> &'static str {
        match self {
            Self::Stage { stage, .. } => stage,
            Self::RecoveryExhausted { source, .. } => source.stage(),
            _ => "unknown",
        }
    }
    fn cause(&self) -> &Self {
        match self {
            Self::Stage { source, .. } | Self::RecoveryExhausted { source, .. } => source.cause(),
            _ => self,
        }
    }
    /// Allowlisted categories only: raw errors may contain URLs, credentials or response data.
    pub fn category(&self) -> &'static str {
        match self.cause() {
            Self::Http(e) if e.is_timeout() => "timeout",
            Self::Http(e) if e.is_connect() => "connect",
            Self::Http(e) if e.status().is_some() => "http_status",
            Self::Http(_) => "http_transport",
            Self::Timeout(_) => "timeout",
            Self::WebSocket(tokio_tungstenite::tungstenite::Error::Io(_)) => "socket_io",
            Self::WebSocket(
                tokio_tungstenite::tungstenite::Error::ConnectionClosed
                | tokio_tungstenite::tungstenite::Error::AlreadyClosed,
            ) => "socket_closed",
            Self::WebSocket(tokio_tungstenite::tungstenite::Error::Http(_)) => "ws_http_status",
            Self::WebSocket(_) => "websocket",
            Self::Io(_) => "io",
            Self::Json(_) => "invalid_data",
            Self::Code(t)
                if matches!(
                    t.code.as_str(),
                    "auth.tfaRequired" | "auth.tfaBindingRequired"
                ) =>
            {
                "two_factor_required"
            }
            Self::Code(t) if t.code == "auth.nasRejected" => "nas_rejected",
            Self::Code(t) if t.code == "auth.authClosed" => "socket_closed",
            Self::Code(t) if t.code == "auth.authDisconnected" => "socket_disconnected",
            _ => "protocol",
        }
    }
    pub fn retryable(&self) -> bool {
        match self {
            Self::RecoveryExhausted { .. } => return false,
            Self::Stage { source, .. } => return source.retryable(),
            _ => {}
        }
        use tokio_tungstenite::tungstenite::Error as WsError;
        match self.cause() {
            Self::Timeout(_) => true,
            Self::Http(e) => {
                e.is_timeout()
                    || e.is_connect()
                    || e.status()
                        .is_some_and(|s| s.is_server_error() || s.as_u16() == 429)
                    || (e.status().is_none()
                        && !e.is_builder()
                        && !e.is_redirect()
                        && !e.is_decode())
            }
            Self::WebSocket(
                WsError::Io(_) | WsError::ConnectionClosed | WsError::AlreadyClosed,
            ) => true,
            Self::WebSocket(WsError::Protocol(
                tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
            )) => true,
            Self::WebSocket(WsError::Http(response)) => {
                response.status().is_server_error() || response.status().as_u16() == 429
            }
            Self::Io(e) => matches!(
                e.kind(),
                std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::ConnectionAborted
                    | std::io::ErrorKind::ConnectionRefused
                    | std::io::ErrorKind::NotConnected
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::Interrupted
            ),
            Self::Code(t)
                if matches!(
                    t.code.as_str(),
                    "auth.entryHandshakeFailed" | "auth.ticketExchangeFailed"
                ) =>
            {
                t.params
                    .get("status")
                    .and_then(|s| s.parse::<u16>().ok())
                    .is_some_and(|s| s >= 500 || s == 429)
            }
            Self::Code(t) => matches!(
                t.code.as_str(),
                "auth.authClosed"
                    | "auth.authDisconnected"
                    | "resolver.scriptMissing"
                    | "resolver.signatureChanged"
                    | "resolver.relayMissing"
                    | "resolver.resolveFailed"
            ),
            _ => false,
        }
    }
    pub fn diagnostic(&self, attempt: usize) -> Text {
        let text = self.text();
        // Only numeric NAS/status/close codes are permitted in diagnostic parameters.
        let status = match self.cause() {
            Self::Http(e) => e.status().map(|s| s.as_u16()),
            Self::WebSocket(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                Some(response.status().as_u16())
            }
            _ => None,
        };
        let detail = status
            .map(|s| s.to_string())
            .or_else(|| {
                ["code", "status", "closeCode"]
                    .into_iter()
                    .filter_map(|key| text.params.get(key))
                    .find_map(|value| value.parse::<i64>().ok())
                    .map(|n| n.to_string())
            })
            .unwrap_or_default();
        Text::with(
            "logs.recoveryError",
            [
                ("stage", self.stage().to_owned()),
                ("kind", self.category().to_owned()),
                ("code", text.code),
                ("detail", detail),
                ("attempt", attempt.to_string()),
            ],
        )
    }
    /// Locale-neutral description for the UI; the frontend translates it.
    pub fn text(&self) -> Text {
        match self {
            AppError::Stage { source, .. } => source.text(),
            AppError::RecoveryExhausted { attempts, .. } => {
                Text::with("recovery.exhausted", [("max", attempts.to_string())])
            }
            AppError::Code(text) => text.clone(),
            AppError::Http(_) => Text::new("error.http"),
            AppError::WebSocket(_) => Text::new("error.webSocket"),
            AppError::Io(error) => Text::with("error.io", [("detail", error.to_string())]),
            AppError::Json(_) => Text::new("error.invalidData"),
            AppError::Timeout(_) => Text::new("error.timeout"),
        }
    }
}
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        self.text().serialize(serializer)
    }
}
pub type Result<T> = std::result::Result<T, AppError>;
pub fn error(code: &str) -> AppError {
    AppError::Code(Text::new(code))
}
pub fn error_with<const N: usize>(code: &str, params: [(&str, String); N]) -> AppError {
    AppError::Code(Text::with(code, params))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagnostics_never_include_raw_error_data() {
        let e = AppError::Io(std::io::Error::other(
            "fixture-password token=fixture-secret https://fixture.test/private",
        ))
        .at("resolve");
        let json = serde_json::to_string(&e.diagnostic(2)).unwrap();
        for private in [
            "fixture-password",
            "fixture-secret",
            "fixture.test",
            "private",
        ] {
            assert!(!json.contains(private));
        }
        assert!(json.contains("resolve"));
        let e = error_with(
            "auth.nasRejected",
            [
                ("code", "1000".into()),
                ("secret", "fixture-private".into()),
            ],
        )
        .at("user_login");
        let json = serde_json::to_string(&e.diagnostic(1)).unwrap();
        assert!(json.contains("1000"));
        assert!(!json.contains("fixture-private"));
    }
    #[test]
    fn account_denials_and_protocol_errors_are_not_transient() {
        for code in [
            "auth.nasRejected",
            "auth.tfaRequired",
            "auth.tfaBindingRequired",
            "auth.credentialRefreshInvalid",
            "auth.entryTokenRefreshFailed",
        ] {
            assert!(!error(code).at("user_login").retryable());
        }
        assert!(error("resolver.scriptMissing").at("resolve").retryable());
        assert!(error("auth.authClosed").retryable());
        assert!(error("auth.authDisconnected").retryable());
    }
    #[test]
    fn handshake_server_failures_retry_but_authorization_failures_do_not() {
        for code in ["auth.entryHandshakeFailed", "auth.ticketExchangeFailed"] {
            for status in [429, 500, 502, 503, 504] {
                assert!(error_with(code, [("status", status.to_string())]).retryable());
            }
            for status in [400, 401, 403, 404] {
                assert!(!error_with(code, [("status", status.to_string())]).retryable());
            }
        }
    }
    #[test]
    fn abrupt_websocket_disconnect_is_retryable() {
        let e = AppError::WebSocket(tokio_tungstenite::tungstenite::Error::Protocol(
            tokio_tungstenite::tungstenite::error::ProtocolError::ResetWithoutClosingHandshake,
        ));
        assert!(e.retryable());
    }
    #[tokio::test]
    async fn http_status_classification_and_logs_are_safe() {
        use axum::{routing::get, Router};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new()
                    .route(
                        "/server",
                        get(|| async { axum::http::StatusCode::SERVICE_UNAVAILABLE }),
                    )
                    .route(
                        "/denied",
                        get(|| async { axum::http::StatusCode::FORBIDDEN }),
                    ),
            )
            .await
            .unwrap();
        });
        for (path, retry) in [("server", true), ("denied", false)] {
            let e = reqwest::get(format!("http://{address}/{path}?token=fixture-secret"))
                .await
                .unwrap()
                .error_for_status()
                .unwrap_err();
            let e = AppError::from(e).at("resolve");
            assert_eq!(e.retryable(), retry);
            assert_eq!(
                e.diagnostic(1).params["detail"],
                if retry { "503" } else { "403" }
            );
            let log = serde_json::to_string(&e.diagnostic(1)).unwrap();
            assert!(!log.contains("fixture-secret"));
            assert!(!log.contains(&address.to_string()));
        }
        server.abort();
    }
}
