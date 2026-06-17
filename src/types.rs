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

/// Present in [`ScanResult::ofac`] when the address is on the OFAC SDN list.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfacMatch {
    pub name:            String,
    pub program:         String,
    pub chain_code:      String,
    /// `"direct"` = scanned address itself; `"counterparty"` = 1-hop tx partner.
    #[serde(rename = "type")]
    pub match_type:      String,
    pub matched_address: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanResult {
    pub result: Option<bool>,
    pub brain_key: Option<String>,
    pub free_scans_left: Option<u32>,
    pub source: Option<String>,
    pub count: Option<u32>,
    /// Set when the address (or a direct counterparty) is on the OFAC SDN list.
    pub ofac: Option<OfacMatch>,
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

// ── Natural language jobs ──────────────────────────────────────────────────

/// Options for submitting a natural language job.
#[derive(Debug, Clone, Default)]
pub struct AskOptions {
    /// Budget in POH (e.g. 0.5 = 0.5 POH). Required for skill jobs.
    pub budget: f64,
    /// Wallet address to charge from. Required when budget > 0.
    pub wallet_address: Option<String>,
}

impl AskOptions {
    pub fn new(budget: f64) -> Self {
        Self { budget, wallet_address: None }
    }

    pub fn wallet(mut self, addr: impl Into<String>) -> Self {
        self.wallet_address = Some(addr.into());
        self
    }
}

/// Reference returned immediately after submitting a job.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskJobRef {
    pub job_id: String,
    pub status: String,
    pub status_url: Option<String>,
    pub result_url: Option<String>,
    pub message: Option<String>,
}

/// Lightweight status snapshot for a natural language job.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskJobStatus {
    pub job_id: String,
    /// `"queued"` | `"computing"` | `"done"` | `"error"`
    pub status: String,
    pub error: Option<String>,
    pub updated_at: Option<String>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskJobResultRaw {
    pub job_id: String,
    pub status: Option<String>,
    pub profile: Option<AskJobProfile>,
    pub error: Option<String>,
}

#[doc(hidden)]
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskJobProfile {
    pub skill_output: Option<serde_json::Value>,
    pub nl_response: Option<String>,
    pub skill_id: Option<String>,
    pub tokens_used: Option<u32>,
}

/// Final result returned after a natural language job completes.
#[derive(Debug, Clone)]
pub struct AskJobResult {
    pub job_id: String,
    pub status: String,
    /// Raw skill output. Shape is skill-dependent.
    pub output: Option<serde_json::Value>,
    /// Natural language answer generated by the miner's LLM.
    /// Present when the job payload included a `question` field.
    pub nl_response: Option<String>,
    pub skill_id: Option<String>,
    pub tokens_used: Option<u32>,
    pub error: Option<String>,
}

impl From<AskJobResultRaw> for AskJobResult {
    fn from(r: AskJobResultRaw) -> Self {
        Self {
            status:      r.status.unwrap_or_else(|| "done".to_owned()),
            output:      r.profile.as_ref().and_then(|p| p.skill_output.clone()),
            nl_response: r.profile.as_ref().and_then(|p| p.nl_response.clone()),
            skill_id:    r.profile.as_ref().and_then(|p| p.skill_id.clone()),
            tokens_used: r.profile.as_ref().and_then(|p| p.tokens_used),
            error:       r.error,
            job_id:      r.job_id,
        }
    }
}

// ── Node info ──────────────────────────────────────────────────────────────

/// Metadata about a PoH miner node.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeInfo {
    pub status: String,
    pub node_id: Option<String>,
    pub version: Option<String>,
    pub wallet: Option<String>,
    pub reputation: Option<f64>,
    pub uptime: Option<u64>,
    pub peers: Option<u32>,
}

// ── Skills ─────────────────────────────────────────────────────────────────

/// A skill available on the network.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Skill {
    pub id: String,
    pub version: Option<String>,
    pub description: Option<String>,
    pub triggers: Option<Vec<String>>,
    pub fee_min: Option<u64>,
}

// ── Wallet / blockchain ────────────────────────────────────────────────────

#[derive(Debug, Clone, Deserialize)]
pub struct WalletBalance {
    pub address: String,
    /// Balance in μPOH (1 POH = 1_000_000_000 μPOH).
    pub balance: i64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AccountNonce {
    pub address: String,
    pub nonce: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TxHistoryEntry {
    pub height: i64,
    pub delta: i64,
    pub tx_hash: String,
    pub ts: i64,
    pub label: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxHistoryResult {
    pub address: String,
    pub entries: Vec<TxHistoryEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PohTx {
    pub from: String,
    pub to: String,
    /// Amount in μPOH.
    pub amount: i64,
    pub fee: i64,
    pub nonce: i64,
    pub timestamp: i64,
    pub memo: String,
    #[serde(rename = "txHash", skip_serializing_if = "Option::is_none")]
    pub tx_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(rename = "signingPublicKey", skip_serializing_if = "Option::is_none")]
    pub signing_public_key: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TxSubmitResult {
    pub ok: bool,
    pub tx_hash: String,
    pub queue_size: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTxResult {
    pub txs: Vec<serde_json::Value>,
    pub count: i64,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MinerInfo {
    pub miner_address: String,
    pub gas_price: i64,
    pub model: String,
    pub queue_length: i64,
    pub reputation: f64,
}

#[derive(Debug, Clone)]
pub struct KeyPair {
    /// PKCS8 PEM private key. Keep secret.
    pub signing_private_key: String,
    /// SPKI PEM public key. Register with the node via `register_signing_key()`.
    pub signing_public_key: String,
}
