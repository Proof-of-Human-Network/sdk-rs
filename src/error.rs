use thiserror::Error;

#[derive(Debug, Error)]
pub enum PohError {
    /// HTTP non-2xx. `body` is the parsed JSON object when available
    /// (e.g. 412 `HF_DATASET_DOWNLOAD_REQUIRED` with `datasetId`).
    #[error("HTTP {status}: {message}")]
    Api {
        status: u16,
        message: String,
        body: Option<serde_json::Value>,
    },

    #[error("Request timed out")]
    Timeout,

    #[error("Network error: {0}")]
    Network(#[from] reqwest::Error),

    #[error("Poll timeout: job did not complete within the allowed duration")]
    PollTimeout,

    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    #[error("No POH node available — all candidates failed to respond")]
    NoNodeAvailable,
}

impl PohError {
    pub fn api(status: u16, message: impl Into<String>, body: Option<serde_json::Value>) -> Self {
        Self::Api {
            status,
            message: message.into(),
            body,
        }
    }
}

pub type Result<T> = std::result::Result<T, PohError>;
