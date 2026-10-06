use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("{0}")]
    Message(String),
    #[error("网络请求失败，请检查网络或 FN Connect 状态")]
    Http(#[from] reqwest::Error),
    #[error("WebSocket 连接失败，请检查 NAS 是否在线")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),
    #[error("本地文件或监听端口操作失败：{0}")]
    Io(#[from] std::io::Error),
    #[error("NAS 返回了无法识别的数据")]
    Json(#[from] serde_json::Error),
    #[error("操作超时，请检查 NAS 网络状态")]
    Timeout(#[from] tokio::time::error::Elapsed),
}
impl Serialize for AppError {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}
pub type Result<T> = std::result::Result<T, AppError>;
pub fn error(message: impl Into<String>) -> AppError {
    AppError::Message(message.into())
}
