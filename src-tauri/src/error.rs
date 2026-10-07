use crate::text::Text;
use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Code(Text),
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
    /// Locale-neutral description for the UI; the frontend translates it.
    pub fn text(&self) -> Text {
        match self {
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
