use thiserror::Error;

#[derive(Debug, Error)]
pub enum ProxyError {
    #[error("proxy config error: {0}")]
    ConfigError(String),
    #[error("proxy auth error: {0}")]
    AuthError(String),
    #[error("proxy request failed: {0}")]
    RequestFailed(String),
    #[error("proxy upstream returned {status}: {body}")]
    UpstreamError { status: u16, body: String },
    #[error("proxy transform error: {0}")]
    TransformError(String),
}
