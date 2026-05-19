# poh-sdk

Rust SDK for the [Proof of Human](https://proofofhuman.ge) API.

## Add to your project

```toml
[dependencies]
poh-sdk = "0.1"
tokio   = { version = "1", features = ["full"] }
```

## Quick start

```rust
use poh_sdk::{PohClient, PohClientOptions, ScanOptions};

#[tokio::main]
async fn main() -> poh_sdk::Result<()> {
    let poh = PohClient::new(
        PohClientOptions::new("https://proofofhuman.ge")
            .api_key("your-api-key"),
    );

    let res = poh.scan("0xabc...", ScanOptions::default()).await?;

    match res.result {
        Some(true)  => println!("Human ✓"),
        Some(false) => println!("Bot ✗"),
        None        => println!("Inconclusive"),
    }

    Ok(())
}
```

## Bulk scanning

```rust
use poh_sdk::{PohClient, PohClientOptions, ScanOptions, PollOptions};

#[tokio::main]
async fn main() -> poh_sdk::Result<()> {
    let poh = PohClient::new(PohClientOptions::new("https://proofofhuman.ge"));

    let done = poh.scan_and_wait(
        &["0xaaa", "0xbbb", "0xccc"],
        ScanOptions::default(),
        PollOptions {
            on_progress: Some(|job| println!("{:.0}% ({}/{})", job.percent, job.done, job.total)),
            ..Default::default()
        },
    ).await?;

    for item in &done.results {
        println!("{} → {:?}", item.input, item.result);
    }

    Ok(())
}
```

## Step-by-step bulk flow

```rust
// 1. Submit
let bulk = poh.scan_bulk(&["0xaaa", "0xbbb"], ScanOptions::default()).await?;
println!("Job: {}", bulk.job_id);

// 2. Poll until done
let done = poh.poll_job(&bulk.job_id, PollOptions::default()).await?;
assert!(done.is_terminal());
```

## Signal methods

```rust
// List all available on-chain signal methods
let methods = poh.get_methods(None).await?;
for m in &methods {
    println!("{} ({}) — score {}", m.id, m.method_type, m.score);
}

// Fetch a specific method
let method = poh.get_method("method-id").await?;
```

## Brain verdict

```rust
let scan = poh.scan("0xabc...", ScanOptions::default()).await?;

if let Some(key) = scan.brain_key {
    let verdict = poh.get_brain_verdict(&key).await?;
    println!("confidence: {:?}", verdict.confidence);
    println!("reasoning:  {:?}", verdict.reasoning);
}
```

## Client options

```rust
use std::time::Duration;

let poh = PohClient::new(
    PohClientOptions::new("https://proofofhuman.ge")
        .api_key("sk-...")           // paid tier
        .wallet_address("Abc123...") // free-tier tracking
        .timeout(Duration::from_secs(60)),
);
```

## Poll options

```rust
use std::time::Duration;

let opts = PollOptions {
    interval:    Duration::from_secs(2),
    timeout:     Duration::from_secs(300),
    on_progress: Some(|job| println!("{}%", job.percent)),
};
```

## Scan options

```rust
let opts = ScanOptions {
    chain_ids: Some(vec!["1".into(), "137".into()]),  // restrict to chains
    tx_hash:   Some("0xdeadbeef...".into()),           // verify a specific tx
};
```

## Error handling

All methods return `poh_sdk::Result<T>` — a type alias for `std::result::Result<T, PohError>`.

```rust
use poh_sdk::PohError;

match poh.scan("0xabc", ScanOptions::default()).await {
    Ok(res)                              => println!("{:?}", res.result),
    Err(PohError::Api { status, message }) => eprintln!("API {status}: {message}"),
    Err(PohError::PollTimeout)           => eprintln!("job timed out"),
    Err(e)                               => eprintln!("error: {e}"),
}
```

| Variant | When |
|---------|------|
| `PohError::Api { status, message }` | Server returned a non-2xx response |
| `PohError::Network(e)` | Transport/connection failure |
| `PohError::Timeout` | Per-request timeout exceeded |
| `PohError::PollTimeout` | `poll_job` deadline exceeded |
| `PohError::InvalidArgument(msg)` | Bad input (e.g. empty address list) |

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `tokio` | ✓ | Enables `poll_job` / `scan_and_wait` (requires `tokio` runtime) |

To use without the `tokio` feature (raw async without polling helpers):

```toml
poh-sdk = { version = "0.1", default-features = false }
```

## License

MIT
