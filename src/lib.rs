mod error;
mod types;

pub use error::{PohError, Result};
pub use types::*;

use reqwest::{Client, RequestBuilder};
use serde::Serialize;
use std::time::Duration;

#[cfg(feature = "tokio")]
use tokio::time::sleep;

// ── Client options ────────────────────────────────────────────────────────────

/// Configuration for [`PohClient`].
#[derive(Debug, Clone)]
pub struct PohClientOptions {
    /// Base URL of the POH API, e.g. `"https://proofofhuman.ge"`.
    pub base_url: String,
    /// API key for paid tier.
    pub api_key: Option<String>,
    /// Solana wallet address for free-tier request tracking.
    pub wallet_address: Option<String>,
    /// Per-request timeout. Default: 30 s.
    pub timeout: Duration,
}

impl PohClientOptions {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url:       base_url.into().trim_end_matches('/').to_owned(),
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

// ── Client ────────────────────────────────────────────────────────────────────

/// Async Proof of Human API client.
///
/// # Example
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
#[derive(Debug, Clone)]
pub struct PohClient {
    opts:   PohClientOptions,
    http:   Client,
}

impl PohClient {
    pub fn new(opts: PohClientOptions) -> Self {
        let http = Client::builder()
            .timeout(opts.timeout)
            .build()
            .expect("failed to build reqwest client");
        Self { opts, http }
    }

    // ── Request helper ────────────────────────────────────────────────────────

    fn req(&self, method: reqwest::Method, path: &str) -> RequestBuilder {
        let url = format!("{}{}", self.opts.base_url, path);
        let mut rb = self.http.request(method, url);
        if let Some(key) = &self.opts.api_key {
            rb = rb.header("x-api-key", key);
        }
        rb
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
    ///
    /// Returns `result: Some(true)` for human, `Some(false)` for not-human,
    /// `None` for inconclusive.
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
        self.send(self.req(reqwest::Method::POST, "/checker").json(&body)).await
    }

    /// Submit a bulk scan for multiple addresses.
    ///
    /// Returns a [`BulkScanResult`] with a `job_id`; use
    /// [`poll_job`](Self::poll_job) to retrieve results.
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
        self.send(self.req(reqwest::Method::POST, "/checker").json(&body)).await
    }

    // ── Job polling ───────────────────────────────────────────────────────────

    /// Fetch the current status of an async scan job.
    pub async fn get_job(&self, job_id: &str) -> Result<JobStatus> {
        let path = format!("/checker/job/{}", urlencoding::encode(job_id));
        self.send(self.req(reqwest::Method::GET, &path)).await
    }

    /// Poll a job until it reaches `done` or `error`, then return the final status.
    ///
    /// # Example
    /// ```no_run
    /// # use poh_sdk::{PohClient, PohClientOptions, PollOptions};
    /// # #[tokio::main] async fn main() {
    /// # let poh = PohClient::new(PohClientOptions::new("http://localhost:3000"));
    /// let opts = PollOptions { on_progress: Some(|j| println!("{}%", j.percent)), ..Default::default() };
    /// let done = poh.poll_job("job-id", opts).await.unwrap();
    /// # }
    /// ```
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
        self.send(self.req(reqwest::Method::GET, &path)).await
    }

    /// Poll the brain verdict until status leaves `"pending"`, then return it.
    ///
    /// # Example
    /// ```no_run
    /// # use poh_sdk::{PohClient, PohClientOptions, BrainPollOptions};
    /// # #[tokio::main] async fn main() {
    /// # let poh = PohClient::new(PohClientOptions::new("http://localhost:3000"));
    /// let scan    = poh.scan("0xabc...", Default::default()).await.unwrap();
    /// let verdict = poh.poll_brain_verdict(scan.brain_key.as_deref().unwrap(), Default::default()).await.unwrap();
    /// println!("{:?}", verdict.verdict);
    /// # }
    /// ```
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
    ///
    /// Returns both the raw [`ScanResult`] and the resolved [`BrainVerdict`].
    ///
    /// # Example
    /// ```no_run
    /// # use poh_sdk::{PohClient, PohClientOptions};
    /// # #[tokio::main] async fn main() {
    /// # let poh = PohClient::new(PohClientOptions::new("http://localhost:3000"));
    /// let sv = poh.scan_and_verdict("0xabc...", Default::default(), Default::default()).await.unwrap();
    /// println!("{:?} ({:?})", sv.verdict.verdict, sv.verdict.confidence);
    /// # }
    /// ```
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
        self.send(self.req(reqwest::Method::GET, &format!("/verifyer{qs}"))).await
    }

    /// Get a single method by ID.
    pub async fn get_method(&self, method_id: &str) -> Result<Method> {
        let path = format!("/verifyer/{}", urlencoding::encode(method_id));
        self.send(self.req(reqwest::Method::GET, &path)).await
    }
}
