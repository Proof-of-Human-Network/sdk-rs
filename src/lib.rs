mod error;
mod types;

pub use error::{PohError, Result};
pub use types::*;

use reqwest::{Client, RequestBuilder};
use serde::Serialize;
use std::sync::Arc;
use std::time::Duration;

#[cfg(feature = "tokio")]
use tokio::{sync::OnceCell, time::sleep};

// ── Default nodes ──────────────────────────────────────────────────────────────

pub const DEFAULT_NODES: &[&str] = &[
    "https://bootnode.proofofhuman.ge",
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
        }
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

    // ── Request helper ────────────────────────────────────────────────────────

    async fn req(&self, method: reqwest::Method, path: &str) -> Result<RequestBuilder> {
        let base = self.base_url().await?;
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
            let status  = res.status().as_u16();
            let body    = res.text().await.unwrap_or_default();
            let message = serde_json::from_str::<serde_json::Value>(&body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_owned))
                .unwrap_or(body);
            return Err(PohError::Api { status, message });
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
}
