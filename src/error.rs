use thiserror::Error;

#[derive(Debug, Error)]
pub enum PohError {
    #[error("HTTP {status}: {message}")]
    Api { status: u16, message: String },

    #[error("Request timed out")]
    Timeout,

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Poll timeout: job did not complete within the allowed duration")]
    PollTimeout,

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
}

pub type Result<T> = std::result::Result<T, PohError>;
