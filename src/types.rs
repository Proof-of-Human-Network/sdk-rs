use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Duration;

// ── Scan ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chain_ids: Option<Vec<String>>,
    #[serde(rename = "txHash", skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub result: Option<bool>,
    pub brain_key: Option<String>,
    pub free_scans_left: Option<u32>,
    pub source: Option<String>,
    pub count: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BulkScanResult {
    pub job_id: String,
    pub status: JobStatusCode,
    pub total: u32,
    pub poll_url: String,
    pub free_scans_left: Option<u32>,
}

// ── Jobs ──────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobStatusCode {
    Queued,
    Processing,
    Done,
    Error,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResultItem {
    pub input: String,
    pub result: Option<bool>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobStatus {
    pub job_id: String,
    pub status: JobStatusCode,
    pub total: u32,
    pub done: u32,
    pub percent: f32,
    pub results: Vec<ScanResultItem>,
    pub errors: Vec<String>,
    pub created_at: String,
    pub completed_at: Option<String>,
}

impl JobStatus {
    pub fn is_terminal(&self) -> bool {
        matches!(self.status, JobStatusCode::Done | JobStatusCode::Error)
    }
}

// ── Brain verdict ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct BrainVerdict {
    pub status: String,
    /// `"HUMAN"` | `"AI"` | `"UNCERTAIN"` — `None` while pending.
    pub verdict: Option<String>,
    pub confidence: Option<f32>,
    pub signals: Option<HashMap<String, f32>>,
    pub reasoning: Option<String>,
}

/// Options for [`PohClient::poll_brain_verdict`].
#[derive(Debug, Clone)]
pub struct BrainPollOptions {
    /// Delay between verdict checks. Default: 1.5 s.
    pub interval: Duration,
    /// Maximum total wait time. Default: 30 s.
    pub timeout: Duration,
}

impl Default for BrainPollOptions {
    fn default() -> Self {
        Self {
            interval: Duration::from_millis(1_500),
            timeout:  Duration::from_secs(30),
        }
    }
}

/// Combined result of [`PohClient::scan_and_verdict`].
#[derive(Debug, Clone)]
pub struct ScanWithVerdict {
    pub scan:    ScanResult,
    pub verdict: BrainVerdict,
}

// ── Methods ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Method {
    pub id: String,
    #[serde(rename = "type")]
    pub method_type: String,
    pub description: String,
    pub address: Option<String>,
    pub method: Option<String>,
    pub score: f64,
    pub vote_count: Option<u32>,
    pub chain_id: Option<String>,
}
