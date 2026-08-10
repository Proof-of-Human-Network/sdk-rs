mod error;
mod types;
#[cfg(feature = "signing")]
pub mod signing;
#[cfg(feature = "chatcrypto")]
pub mod chatcrypto;

pub use error::{PohError, Result};
pub use types::*;
#[cfg(feature = "signing")]
pub use signing::{
    generate_key_pair, derive_address_from_signing_key, sign_data, create_signing_proof,
    create_rotation_proof, build_transfer, sign_transaction, compute_tx_hash,
    generate_job_id, compute_job_payment_hash, sign_job_payment,
};
#[cfg(feature = "chatcrypto")]
pub use chatcrypto::{
    derive_encryption_keypair, open, seal, EncryptionKeypair, SealedEnvelope,
};

use reqwest::{Client, RequestBuilder};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "tokio")]
use tokio::{sync::OnceCell, time::sleep};

// ── Default nodes ──────────────────────────────────────────────────────────────

pub const DEFAULT_NODES: &[&str] = &[
    "https://miner.poh.ge",
    "https://proofofhuman.ge",
    "https://poh.assetux.com",
];

// ── Client options ────────────────────────────────────────────────────────────

/// Configuration for [`PohClient`].
#[derive(Debug, Clone)]
pub struct PohClientOptions {
    /// Fixed base URL. Use this OR `nodes`, not both.
    pub base_url: Option<String>,
    /// Candidate node URLs for automatic first-alive discovery.
    /// Defaults to [`DEFAULT_NODES`] when both `base_url` and `nodes` are empty.
    pub nodes: Vec<String>,
    /// API key for paid tier.
    pub api_key: Option<String>,
    /// Solana wallet address for free-tier request tracking.
    pub wallet_address: Option<String>,
    /// Per-request timeout. Default: 30 s.
    pub timeout: Duration,
    /// Local miner URL for state-changing requests (wallet, tx, jobs).
    pub local_base_url: Option<String>,
}

impl PohClientOptions {
    /// Single fixed node — backward-compatible constructor.
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url:       Some(base_url.into().trim_end_matches('/').to_owned()),
            nodes:          vec![],
            api_key:        None,
            wallet_address: None,
            timeout:        Duration::from_secs(30),
            local_base_url: None,
        }
    }

    /// Multi-node constructor — client picks the first responding node.
    pub fn with_nodes(nodes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        Self {
            base_url:       None,
            nodes:          nodes.into_iter()
                                 .map(|n| n.into().trim_end_matches('/').to_owned())
                                 .collect(),
            api_key:        None,
            wallet_address: None,
            timeout:        Duration::from_secs(30),
            local_base_url: None,
        }
    }

    pub fn local_base_url(mut self, url: impl Into<String>) -> Self {
        self.local_base_url = Some(url.into().trim_end_matches('/').to_owned());
        self
    }

    pub fn api_key(mut self, key: impl Into<String>) -> Self {
        self.api_key = Some(key.into());
        self
    }

    pub fn wallet_address(mut self, addr: impl Into<String>) -> Self {
        self.wallet_address = Some(addr.into());
        self
    }

    pub fn timeout(mut self, d: Duration) -> Self {
        self.timeout = d;
        self
    }
}

impl Default for PohClientOptions {
    fn default() -> Self {
        Self::with_nodes(DEFAULT_NODES.iter().copied())
    }
}

// ── Poll options ──────────────────────────────────────────────────────────────

/// Options for [`PohClient::poll_job`].
#[derive(Debug, Clone)]
pub struct PollOptions {
    /// Delay between status checks. Default: 1.5 s.
    pub interval: Duration,
    /// Maximum total wait time. Default: 120 s.
    pub timeout: Duration,
    /// Optional callback fired on every status update.
    pub on_progress: Option<fn(&JobStatus)>,
}

impl Default for PollOptions {
    fn default() -> Self {
        Self {
            interval:    Duration::from_millis(1_500),
            timeout:     Duration::from_secs(120),
            on_progress: None,
        }
    }
}

// ── Node discovery helpers ────────────────────────────────────────────────────

#[cfg(feature = "tokio")]
async fn probe_node(http: &Client, node: &str) -> bool {
    let url = format!("{}/healthz", node);
    http.head(&url)
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .is_ok()
}

#[cfg(feature = "tokio")]
async fn pick_first_alive(http: &Client, nodes: &[String]) -> Option<String> {
    for node in nodes {
        if probe_node(http, node).await {
            return Some(node.clone());
        }
    }
    None
}

// ── Client ────────────────────────────────────────────────────────────────────

/// Async Proof of Human API client.
///
/// # Example — default nodes (multi-node discovery)
/// ```no_run
/// use poh_sdk::{PohClient, PohClientOptions};
///
/// #[tokio::main]
/// async fn main() {
///     let poh = PohClient::new(PohClientOptions::default());
///     let res = poh.scan("0xabc...", Default::default()).await.unwrap();
///     println!("{:?}", res.result);
/// }
/// ```
///
/// # Example — single fixed node
/// ```no_run
/// use poh_sdk::{PohClient, PohClientOptions};
///
/// #[tokio::main]
/// async fn main() {
///     let poh = PohClient::new(PohClientOptions::new("https://proofofhuman.ge"));
///     let res = poh.scan("0xabc...", Default::default()).await.unwrap();
///     println!("{:?}", res.result);
/// }
/// ```
#[derive(Clone)]
pub struct PohClient {
    opts:         PohClientOptions,
    http:         Client,
    #[cfg(feature = "tokio")]
    resolved_url: Arc<OnceCell<String>>,
}

impl std::fmt::Debug for PohClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PohClient")
            .field("base_url", &self.opts.base_url)
            .field("nodes", &self.opts.nodes)
            .finish()
    }
}

impl PohClient {
    pub fn new(opts: PohClientOptions) -> Self {
        let http = Client::builder()
            .timeout(opts.timeout)
            .build()
            .expect("failed to build reqwest client");
        Self {
            #[cfg(feature = "tokio")]
            resolved_url: Arc::new(OnceCell::new()),
            opts,
            http,
        }
    }

    // ── Node resolution ───────────────────────────────────────────────────────

    #[cfg(feature = "tokio")]
    async fn base_url(&self) -> Result<String> {
        self.resolved_url.get_or_try_init(|| async {
            if let Some(url) = &self.opts.base_url {
                return Ok(url.clone());
            }
            let nodes = if self.opts.nodes.is_empty() {
                DEFAULT_NODES.iter().map(|s| s.to_string()).collect::<Vec<_>>()
            } else {
                self.opts.nodes.clone()
            };
            pick_first_alive(&self.http, &nodes)
                .await
                .ok_or(PohError::NoNodeAvailable)
        })
        .await
        .cloned()
    }

    #[cfg(not(feature = "tokio"))]
    fn base_url(&self) -> Result<String> {
        self.opts.base_url.clone()
            .ok_or_else(|| PohError::InvalidArgument(
                "base_url required when tokio feature is disabled".into()
            ))
    }

    /// The currently selected node URL, or `None` if discovery has not yet run.
    pub fn active_node(&self) -> Option<String> {
        #[cfg(feature = "tokio")]
        return self.resolved_url.get().cloned();
        #[cfg(not(feature = "tokio"))]
        return self.opts.base_url.clone();
    }

    fn needs_local_node(method: &reqwest::Method, path: &str) -> bool {
        if method == reqwest::Method::GET || method == reqwest::Method::HEAD || method == reqwest::Method::OPTIONS {
            return false;
        }
        let p = path.split('?').next().unwrap_or(path);
        !(method == reqwest::Method::POST && p == "/gossip")
    }

    fn is_loopback(url: &str) -> bool {
        let host = url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or("")
            .split(':')
            .next()
            .unwrap_or("");
        matches!(host, "localhost" | "127.0.0.1" | "::1" | "[::1]")
    }

    async fn resolve_base_url(&self, method: &reqwest::Method, path: &str) -> Result<String> {
        if !Self::needs_local_node(method, path) {
            return self.base_url().await;
        }
        if let Some(local) = &self.opts.local_base_url {
            return Ok(local.clone());
        }
        let remote = self.base_url().await?;
        if Self::is_loopback(&remote) {
            return Ok(remote);
        }
        Err(PohError::api(
            403,
            "This operation requires a local miner node. Set local_base_url in PohClientOptions.",
            None,
        ))
    }

    // ── Request helper ────────────────────────────────────────────────────────

    async fn req(&self, method: reqwest::Method, path: &str) -> Result<RequestBuilder> {
        let base = self.resolve_base_url(&method, path).await?;
        let url  = format!("{}{}", base, path);
        let mut rb = self.http.request(method, url);
        if let Some(key) = &self.opts.api_key {
            rb = rb.header("x-api-key", key);
        }
        Ok(rb)
    }

    async fn send<T: serde::de::DeserializeOwned>(&self, rb: RequestBuilder) -> Result<T> {
        let res = rb.send().await?;
        if !res.status().is_success() {
            let status = res.status().as_u16();
            let text   = res.text().await.unwrap_or_default();
            let parsed = serde_json::from_str::<serde_json::Value>(&text).ok();
            let message = parsed
                .as_ref()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                .unwrap_or(text);
            return Err(PohError::api(status, message, parsed));
        }
        Ok(res.json::<T>().await?)
    }

    // ── Scan ──────────────────────────────────────────────────────────────────

    /// Scan a single wallet address.
    pub async fn scan(
        &self,
        input: &str,
        options: ScanOptions,
    ) -> Result<ScanResult> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            input: &'a str,
            #[serde(skip_serializing_if = "Option::is_none")]
            wallet_address: Option<&'a str>,
            #[serde(flatten)]
            opts: ScanOptions,
        }
        let body = Body {
            input,
            wallet_address: self.opts.wallet_address.as_deref(),
            opts: options,
        };
        self.send(self.req(reqwest::Method::POST, "/checker").await?.json(&body)).await
    }

    /// Submit a bulk scan for multiple addresses.
    pub async fn scan_bulk(
        &self,
        inputs: &[&str],
        options: ScanOptions,
    ) -> Result<BulkScanResult> {
        if inputs.is_empty() {
            return Err(PohError::InvalidArgument("inputs slice must not be empty".into()));
        }
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Body<'a> {
            input: &'a [&'a str],
            #[serde(skip_serializing_if = "Option::is_none")]
            wallet_address: Option<&'a str>,
            #[serde(flatten)]
            opts: ScanOptions,
        }
        let body = Body {
            input: inputs,
            wallet_address: self.opts.wallet_address.as_deref(),
            opts: options,
        };
        self.send(self.req(reqwest::Method::POST, "/checker").await?.json(&body)).await
    }

    // ── Job polling ───────────────────────────────────────────────────────────

    /// Fetch the current status of an async scan job.
    pub async fn get_job(&self, job_id: &str) -> Result<JobStatus> {
        let path = format!("/checker/job/{}", urlencoding::encode(job_id));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Poll a job until it reaches `done` or `error`, then return the final status.
    #[cfg(feature = "tokio")]
    pub async fn poll_job(&self, job_id: &str, opts: PollOptions) -> Result<JobStatus> {
        let deadline = std::time::Instant::now() + opts.timeout;
        loop {
            let job = self.get_job(job_id).await?;
            if let Some(cb) = opts.on_progress {
                cb(&job);
            }
            if job.is_terminal() {
                return Ok(job);
            }
            if std::time::Instant::now() + opts.interval > deadline {
                return Err(PohError::PollTimeout);
            }
            sleep(opts.interval).await;
        }
    }

    /// Convenience: submit a bulk scan and wait for all results.
    #[cfg(feature = "tokio")]
    pub async fn scan_and_wait(
        &self,
        inputs: &[&str],
        scan_opts: ScanOptions,
        poll_opts: PollOptions,
    ) -> Result<JobStatus> {
        let job = self.scan_bulk(inputs, scan_opts).await?;
        self.poll_job(&job.job_id, poll_opts).await
    }

    // ── Brain verdict ─────────────────────────────────────────────────────────

    /// Retrieve the AI brain verdict for a completed scan.
    pub async fn get_brain_verdict(&self, brain_key: &str) -> Result<BrainVerdict> {
        let path = format!("/checker/brain/{}", urlencoding::encode(brain_key));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Poll the brain verdict until status leaves `"pending"`, then return it.
    #[cfg(feature = "tokio")]
    pub async fn poll_brain_verdict(
        &self,
        brain_key: &str,
        opts: BrainPollOptions,
    ) -> Result<BrainVerdict> {
        let deadline = std::time::Instant::now() + opts.timeout;
        loop {
            let v = self.get_brain_verdict(brain_key).await?;
            if v.status != "pending" {
                return Ok(v);
            }
            if std::time::Instant::now() + opts.interval > deadline {
                return Err(PohError::PollTimeout);
            }
            sleep(opts.interval).await;
        }
    }

    /// Convenience: scan a single address and wait for the AI brain verdict.
    #[cfg(feature = "tokio")]
    pub async fn scan_and_verdict(
        &self,
        input: &str,
        scan_opts:  ScanOptions,
        brain_opts: BrainPollOptions,
    ) -> Result<ScanWithVerdict> {
        let scan = self.scan(input, scan_opts).await?;
        let verdict = match scan.brain_key.as_deref() {
            Some(key) => self.poll_brain_verdict(key, brain_opts).await?,
            None => BrainVerdict {
                status:     "not_found".to_owned(),
                verdict:    None,
                confidence: None,
                signals:    None,
                reasoning:  None,
            },
        };
        Ok(ScanWithVerdict { scan, verdict })
    }

    // ── Methods ───────────────────────────────────────────────────────────────

    /// List available signal verification methods, ordered by vote score.
    pub async fn get_methods(&self, wallet_address: Option<&str>) -> Result<Vec<Method>> {
        let addr = wallet_address.or(self.opts.wallet_address.as_deref());
        let qs   = addr
            .map(|a| format!("?address={}", urlencoding::encode(a)))
            .unwrap_or_default();
        self.send(self.req(reqwest::Method::GET, &format!("/verifyer{qs}")).await?).await
    }

    /// Get a single method by ID.
    pub async fn get_method(&self, method_id: &str) -> Result<Method> {
        let path = format!("/verifyer/{}", urlencoding::encode(method_id));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    // ── Natural language jobs ─────────────────────────────────────────────────

    /// Route a natural language question to a skill and submit it as a job.
    ///
    /// Returns immediately with an [`AskJobRef`]; poll with [`poll_job_result`]
    /// or use [`ask_and_wait`] to block until the answer is ready.
    ///
    /// Returns [`PohError::InvalidArgument`] if no skill matches the question.
    ///
    /// [`poll_job_result`]: PohClient::poll_job_result
    /// [`ask_and_wait`]: PohClient::ask_and_wait
    pub async fn submit_job(&self, question: &str, opts: AskOptions) -> Result<AskJobRef> {
        let max_budget = (opts.budget * 1_000_000_000.0) as i64;

        // 1. Route to a skill
        let route_body = serde_json::json!({
            "message": question,
            "budget":  max_budget,
        });
        let route: serde_json::Value = self.send(
            self.req(reqwest::Method::POST, "/chat/route").await?.json(&route_body)
        ).await?;

        let rtype = route.get("type").and_then(|v| v.as_str()).unwrap_or("chat");
        if matches!(rtype, "cascade" | "tasks" | "dataset" | "hf-model" | "sequence") {
            return Err(PohError::InvalidArgument(format!(
                "Route type \"{rtype}\" is free (task cascade / dataset / media). Use chat() instead of submit_job()."
            )));
        }
        if rtype != "skill" {
            let reason = route.get("reason")
                .and_then(|v| v.as_str())
                .unwrap_or("No skill matched the question");
            return Err(PohError::InvalidArgument(reason.to_owned()));
        }
        let skill_id = route.get("skillId")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PohError::InvalidArgument("No skillId in route response".into()))?;
        let input = route.get("input").cloned()
            .unwrap_or_else(|| serde_json::Value::Object(Default::default()));

        // 2. Submit job
        let mut job_body = serde_json::json!({
            "type":      "skill",
            "skillId":   skill_id,
            "payload":   input,
            "maxBudget": max_budget,
        });
        let wallet_address = opts.wallet_address.as_deref()
            .or(self.opts.wallet_address.as_deref());
        if let Some(addr) = wallet_address {
            job_body["requesterAddress"] = serde_json::Value::String(addr.to_owned());
        }

        // Skill jobs always require a fee — sign the payment when budget > 0.
        // No "unverified" fallback: the node rejects the job outright (never runs
        // it) without a valid signed payment proof.
        if max_budget > 0 {
            #[cfg(feature = "signing")]
            {
                let requester = wallet_address.ok_or_else(|| PohError::InvalidArgument(
                    "submit_job: wallet_address is required when budget > 0".into()
                ))?;
                let private_key = opts.private_key_pem.as_deref().ok_or_else(|| PohError::InvalidArgument(
                    "submit_job: private_key_pem is required when budget > 0 — skill jobs always require a signed fee.".into()
                ))?;
                let job_id = crate::signing::generate_job_id();
                job_body["id"] = serde_json::Value::String(job_id.clone());
                let miner_info = self.get_miner_info().await?;
                let nonce_info = self.get_nonce(requester).await?;
                let (tx_hash, signature) = crate::signing::sign_job_payment(
                    &job_id, requester, &miner_info.miner_address, max_budget, nonce_info.nonce, private_key,
                ).map_err(|e| PohError::InvalidArgument(e.to_string()))?;
                job_body["paymentTx"] = serde_json::json!({ "txHash": tx_hash, "signature": signature });
            }
            #[cfg(not(feature = "signing"))]
            {
                return Err(PohError::InvalidArgument(
                    "submit_job: budget > 0 requires the 'signing' crate feature to be enabled".into()
                ));
            }
        }

        self.send(self.req(reqwest::Method::POST, "/job").await?.json(&job_body)).await
    }

    /// Submit a paid compute job that runs a user-specified model (and, optionally,
    /// grounds the answer in a Hugging Face dataset already installed on the node).
    /// Compute jobs are never free — the node rejects the request outright unless
    /// it carries a valid signed fee payment.
    ///
    /// ```no_run
    /// use poh_sdk::{PohClient, PohClientOptions, ComputeOptions};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let poh = PohClient::new(PohClientOptions::default());
    ///     let opts = ComputeOptions::new("llama3.1:8b", 0.5, "poh...", "<pem>")
    ///         .dataset("some-org/some-dataset");
    ///     let ref_ = poh.run_compute("Summarize the top 5 rows", opts).await.unwrap();
    ///     println!("{}", ref_.job_id);
    /// }
    /// ```
    #[cfg(feature = "signing")]
    pub async fn run_compute(&self, prompt: &str, opts: ComputeOptions) -> Result<AskJobRef> {
        if opts.budget <= 0.0 {
            return Err(PohError::InvalidArgument(
                "run_compute: budget must be > 0 — compute jobs always require a fee".into()
            ));
        }
        if prompt.is_empty() && opts.attachments.as_ref().map(|a| a.is_empty()).unwrap_or(true) {
            return Err(PohError::InvalidArgument(
                "run_compute: prompt or attachments required".into(),
            ));
        }
        let max_budget = (opts.budget * 1_000_000_000.0) as i64;
        let job_id = opts.job_id.clone().unwrap_or_else(crate::signing::generate_job_id);

        let miner_info = self.get_miner_info().await?;
        let nonce_info = self.get_nonce(&opts.wallet_address).await?;
        let (tx_hash, signature) = crate::signing::sign_job_payment(
            &job_id, &opts.wallet_address, &miner_info.miner_address,
            max_budget, nonce_info.nonce, &opts.private_key_pem,
        ).map_err(|e| PohError::InvalidArgument(e.to_string()))?;

        let mut payload = serde_json::json!({
            "prompt": if prompt.is_empty() { "Please analyze the attached file(s)." } else { prompt },
        });
        if let Some(h) = &opts.history {
            payload["history"] = serde_json::Value::Array(h.clone());
        }
        if let Some(a) = &opts.attachments {
            payload["attachments"] = serde_json::to_value(a)
                .map_err(|e| PohError::InvalidArgument(e.to_string()))?;
        }
        if opts.route == Some(false) {
            payload["route"] = serde_json::Value::Bool(false);
        }

        let mut job_body = serde_json::json!({
            "id":               job_id,
            "type":             "compute",
            "model":            opts.model,
            "dataset":          opts.dataset,
            "payload":          payload,
            "maxBudget":        max_budget,
            "requesterAddress": opts.wallet_address,
            "paymentTx":        { "txHash": tx_hash, "signature": signature },
        });
        if opts.route == Some(false) {
            job_body["route"] = serde_json::Value::Bool(false);
        }

        self.send(self.req(reqwest::Method::POST, "/job").await?.json(&job_body)).await
    }

    /// Free-form chat via `POST /chat/ask` (no fee). Runs task cascade when needed.
    /// Attachments ≤1 MB: text inlined, images use a vision model path.
    ///
    /// On HTTP 412 with `code: HF_DATASET_DOWNLOAD_REQUIRED`, returns
    /// [`PohError::Api`] whose `body` contains `datasetId` — call
    /// [`download_dataset`] then retry with [`ChatOptions::dataset_id`].
    pub async fn chat(&self, message: &str, opts: ChatOptions) -> Result<ChatResult> {
        if message.is_empty() && opts.attachments.as_ref().map(|a| a.is_empty()).unwrap_or(true) {
            return Err(PohError::InvalidArgument(
                "chat: message or attachments required".into(),
            ));
        }
        let mut body = serde_json::json!({
            "message": if message.is_empty() { "Please analyze the attached file(s)." } else { message },
            "history": opts.history.clone().unwrap_or_default(),
            "private": opts.private,
        });
        if let Some(m) = &opts.model {
            body["model"] = serde_json::Value::String(m.clone());
        }
        if let Some(a) = &opts.attachments {
            body["attachments"] = serde_json::to_value(a)
                .map_err(|e| PohError::InvalidArgument(e.to_string()))?;
        }
        if let Some(id) = &opts.dataset_id {
            body["datasetId"] = serde_json::Value::String(id.clone());
        }
        let req_addr = opts
            .requester_address
            .as_deref()
            .or(self.opts.wallet_address.as_deref());
        if let Some(addr) = req_addr {
            body["requesterAddress"] = serde_json::Value::String(addr.to_owned());
        }
        self.send(self.req(reqwest::Method::POST, "/chat/ask").await?.json(&body))
            .await
    }

    /// List Hugging Face datasets installed on the miner.
    pub async fn list_datasets(&self) -> Result<HfDatasetListResult> {
        self.send(self.req(reqwest::Method::GET, "/api/hf-dataset").await?)
            .await
    }

    /// Download + install a Hugging Face dataset on the miner (row-capped).
    pub async fn download_dataset(&self, dataset_id: &str) -> Result<serde_json::Value> {
        if dataset_id.is_empty() {
            return Err(PohError::InvalidArgument(
                "download_dataset: dataset_id required".into(),
            ));
        }
        let path = format!(
            "/api/hf-dataset/{}/download",
            urlencoding::encode(dataset_id)
        );
        self.send(self.req(reqwest::Method::POST, &path).await?).await
    }

    /// Remove an installed HF dataset from the miner.
    pub async fn delete_dataset(&self, dataset_id: &str) -> Result<serde_json::Value> {
        if dataset_id.is_empty() {
            return Err(PohError::InvalidArgument(
                "delete_dataset: dataset_id required".into(),
            ));
        }
        let path = format!("/api/hf-dataset/{}", urlencoding::encode(dataset_id));
        self.send(self.req(reqwest::Method::DELETE, &path).await?)
            .await
    }

    /// Status of configured MCP servers and their tools.
    pub async fn get_mcp_status(&self) -> Result<McpStatusResult> {
        self.send(self.req(reqwest::Method::GET, "/api/mcp/status").await?)
            .await
    }

    /// Fetch the current status of a natural language job (without the full result).
    pub async fn get_job_status(&self, job_id: &str) -> Result<AskJobStatus> {
        let path = format!("/job/{}/status", urlencoding::encode(job_id));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Fetch the full result of a completed natural language job.
    pub async fn get_job_result(&self, job_id: &str) -> Result<AskJobResult> {
        let path = format!("/job/{}/result", urlencoding::encode(job_id));
        let raw: AskJobResultRaw = self.send(
            self.req(reqwest::Method::GET, &path).await?
        ).await?;
        Ok(raw.into())
    }

    /// Poll a natural language job until `done` or `error`, then return the result.
    #[cfg(feature = "tokio")]
    pub async fn poll_job_result(
        &self,
        job_id: &str,
        opts: PollOptions,
    ) -> Result<AskJobResult> {
        let deadline = std::time::Instant::now() + opts.timeout;
        loop {
            let s = self.get_job_status(job_id).await?;
            if s.status == "done" || s.status == "error" {
                return self.get_job_result(job_id).await;
            }
            if std::time::Instant::now() + opts.interval > deadline {
                return Err(PohError::PollTimeout);
            }
            sleep(opts.interval).await;
        }
    }

    /// Convenience: route, submit, and wait for a natural language job.
    ///
    /// ```no_run
    /// use poh_sdk::{PohClient, PohClientOptions, AskOptions, PollOptions};
    ///
    /// #[tokio::main]
    /// async fn main() {
    ///     let poh = PohClient::new(PohClientOptions::default());
    ///     let res = poh.ask_and_wait(
    ///         "What does vitalik.eth write about on Paragraph?",
    ///         AskOptions::new(0.5).wallet("poh..."),
    ///         PollOptions::default(),
    ///     ).await.unwrap();
    ///     println!("{:?}", res.nl_response.or(res.output.map(|o| o.to_string())));
    /// }
    /// ```
    #[cfg(feature = "tokio")]
    pub async fn ask_and_wait(
        &self,
        question: &str,
        ask_opts:  AskOptions,
        poll_opts: PollOptions,
    ) -> Result<AskJobResult> {
        let job = self.submit_job(question, ask_opts).await?;
        self.poll_job_result(&job.job_id, poll_opts).await
    }

    // ── Node info ─────────────────────────────────────────────────────────────

    /// Fetch metadata about the currently connected node.
    pub async fn get_node_info(&self) -> Result<NodeInfo> {
        self.send(self.req(reqwest::Method::GET, "/healthz").await?).await
    }

    /// List all skills available on the connected node.
    pub async fn list_skills(&self) -> Result<Vec<Skill>> {
        let value: serde_json::Value =
            self.send(self.req(reqwest::Method::GET, "/api/skills").await?).await?;
        let items = value.as_array().cloned().or_else(|| {
            value.get("skills").and_then(|s| s.as_array()).cloned()
        }).unwrap_or_default();
        Ok(items.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect())
    }

    // ── Wallet / blockchain ───────────────────────────────────────────────────

    /// Fetch the μPOH balance for an address.
    pub async fn get_balance(&self, address: &str) -> Result<WalletBalance> {
        let path = format!("/api/wallet/balance?address={}", urlencoding::encode(address));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Fetch the current transaction nonce for an address.
    pub async fn get_nonce(&self, address: &str) -> Result<AccountNonce> {
        let path = format!("/api/wallet/nonce?address={}", urlencoding::encode(address));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Fetch the transaction history for an address.
    pub async fn get_transaction_history(&self, address: &str, limit: usize) -> Result<TxHistoryResult> {
        let path = format!(
            "/api/wallet/history?address={}&limit={}",
            urlencoding::encode(address),
            limit,
        );
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Fetch raw transactions for an address.
    pub async fn get_transactions(&self, address: &str) -> Result<serde_json::Value> {
        let path = format!("/api/wallet/transactions?address={}", urlencoding::encode(address));
        self.send(self.req(reqwest::Method::GET, &path).await?).await
    }

    /// Fetch all pending (unconfirmed) transactions in the node's mempool.
    pub async fn get_pending_transactions(&self) -> Result<PendingTxResult> {
        self.send(self.req(reqwest::Method::GET, "/api/tx/pending").await?).await
    }

    /// Submit a signed [`PohTx`] to the network.
    pub async fn submit_transaction(&self, tx: &PohTx) -> Result<TxSubmitResult> {
        self.send(self.req(reqwest::Method::POST, "/api/tx/submit").await?.json(tx)).await
    }

    /// Register an Ed25519 signing public key (SPKI PEM) for a wallet address.
    ///
    /// `proof` is a base64-encoded Ed25519 signature of the raw address bytes,
    /// produced by the matching private key (see [`create_signing_proof`]).
    ///
    /// [`create_signing_proof`]: crate::signing::create_signing_proof
    pub async fn register_signing_key(
        &self,
        address: &str,
        signing_public_key: &str,
        proof: &str,
        rotation_proof: Option<&str>,
    ) -> Result<serde_json::Value> {
        let mut body = serde_json::json!({
            "address":          address,
            "signingPublicKey": signing_public_key,
            "proof":            proof,
        });
        if let Some(rp) = rotation_proof {
            body["rotationProof"] = serde_json::Value::String(rp.to_string());
        }
        self.send(self.req(reqwest::Method::POST, "/api/wallet/register-key").await?.json(&body)).await
    }

    /// Register a [`KeyPair`] from [`generate_key_pair`].
    #[cfg(feature = "signing")]
    pub async fn register_key_pair(
        &self,
        key_pair: &crate::types::KeyPair,
        rotation_proof: Option<&str>,
    ) -> Result<serde_json::Value> {
        let proof = crate::signing::create_signing_proof(&key_pair.address, &key_pair.signing_private_key)
            .map_err(|e| PohError::InvalidArgument(e.to_string()))?;
        self.register_signing_key(
            &key_pair.address,
            &key_pair.signing_public_key,
            &proof,
            rotation_proof,
        ).await
    }

    /// Fetch metadata about the miner node (gas price, model, queue length, reputation).
    pub async fn get_miner_info(&self) -> Result<MinerInfo> {
        self.send(self.req(reqwest::Method::GET, "/api/miner/info").await?).await
    }

    /// Convenience: build, sign, and submit a POH transfer in one call.
    ///
    /// `amount_poh` is in whole POH units (e.g. `1.5` = 1.5 POH = 1_500_000_000 μPOH).
    /// The nonce is fetched automatically and incremented by 1.
    #[cfg(feature = "signing")]
    pub async fn transfer(
        &self,
        from: &str,
        to: &str,
        amount_poh: f64,
        private_key_pem: &str,
        fee: i64,
        memo: &str,
    ) -> Result<TxSubmitResult> {
        let nonce_resp = self.get_nonce(from).await?;
        let next_nonce = nonce_resp.pending_nonce.unwrap_or(nonce_resp.nonce) + 1;
        let tx = signing::build_transfer(from, to, amount_poh, next_nonce, fee, memo)
            .map_err(|e| PohError::InvalidArgument(e.to_string()))?;
        let signed = signing::sign_transaction(&tx, private_key_pem)
            .map_err(|e| PohError::InvalidArgument(e.to_string()))?;
        self.submit_transaction(&signed).await
    }
}
