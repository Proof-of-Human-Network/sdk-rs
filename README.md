# poh-sdk

Rust SDK for the [Proof of Human](https://proofofhuman.ge) network.

## Add to your project

```toml
[dependencies]
poh-sdk = "0.3"
tokio   = { version = "1", features = ["full"] }

# Enable signing utilities:
poh-sdk = { version = "0.3", features = ["signing"] }
```

## Quick start

```rust
use poh_sdk::{PohClient, PohClientOptions, ScanOptions};

#[tokio::main]
async fn main() -> poh_sdk::Result<()> {
    let poh = PohClient::new(
        PohClientOptions::new("https://bootnode.proofofhuman.ge")
            .api_key("your-api-key"),
    );

    let res = poh.scan("0xabc...", ScanOptions::default()).await?;
    match res.result {
        Some(true)  => println!("Human"),
        Some(false) => println!("Bot"),
        None        => println!("Inconclusive"),
    }
    Ok(())
}
```

## Natural language jobs

Skill jobs always require a fee — set `budget`, `wallet_address`, and
`private_key_pem` on `AskOptions` (requires the `signing` feature) so the SDK
can sign the payment. The node verifies the signature and debits the fee
before it will run the job at all; it rejects the request outright (no job
ever runs) without a valid signed payment.

```rust
use poh_sdk::{PohClient, PohClientOptions, AskOptions};

let poh = PohClient::new(PohClientOptions::new("https://proofofhuman.ge"));

let result = poh.ask_and_wait(
    "What does vitalik.eth write about on Paragraph?",
    AskOptions::new(0.5).wallet("poh...").private_key(my_private_key_pem),
    Default::default(),
).await?;

println!("{:?}", result.output);
if let Some(nl) = result.nl_response { println!("{nl}"); }
```

## Compute jobs (your own model + dataset)

Run inference with a model of your choice, optionally grounded in a Hugging
Face dataset already installed on the node. Like skill jobs, compute jobs are
never free — `run_compute` always signs a fee payment. Requires the `signing`
feature.

```rust
use poh_sdk::{PohClient, PohClientOptions, ComputeOptions};

let poh = PohClient::new(PohClientOptions::new("https://proofofhuman.ge"));

let opts = ComputeOptions::new("llama3.1:8b", 0.5, "poh...", my_private_key_pem)
    .dataset("some-org/some-dataset"); // optional

let job_ref = poh.run_compute("Summarize the top 5 rows", opts).await?;
let result  = poh.poll_job_result(&job_ref.job_id, Default::default()).await?;
println!("{:?}", result.output);
```

Before either of these will work, the wallet's signing key must be registered
with the node once via `register_signing_key()` — the node has no way to
verify a signature for a key it has never seen.

## Wallet / blockchain

```rust
// Balance (μPOH — divide by 1e9 for POH)
let bal = poh.get_balance("poh...").await?;
println!("{} μPOH", bal.balance);

// Nonce (needed before building a transaction)
let nonce = poh.get_nonce("poh...").await?;

// Transaction history
let history = poh.get_transaction_history("poh...", 50).await?;
for entry in &history.entries {
    println!("{}: {} μPOH", entry.tx_hash, entry.delta);
}

// Pending mempool transactions
let pending = poh.get_pending_transactions().await?;
println!("{} pending txs", pending.count);

// Miner info
let info = poh.get_miner_info().await?;
println!("{} reputation={}", info.model, info.reputation);
```

## Signing & transactions

Requires the `signing` feature:

```toml
poh-sdk = { version = "0.3", features = ["signing"] }
```

```rust
use poh_sdk::{generate_key_pair, build_transfer, sign_transaction, create_signing_proof};

// 1. Generate a keypair — address is derived from the signing public key
let kp = generate_key_pair()?;

// 2. Register with your local node (one-time, per node)
poh.register_key_pair(&kp, None).await?;

// 3. Build, sign, and submit a transfer
let nonce_resp = poh.get_nonce(&kp.address).await?;
let tx     = build_transfer(&kp.address, &recipient, 5.0, nonce_resp.nonce + 1, 0, "")?;
let signed = sign_transaction(&tx, &kp.signing_private_key)?;
let result = poh.submit_transaction(&signed).await?;
println!("{}", result.tx_hash);

// One-liner convenience (fetches nonce automatically)
let result = poh.transfer(&kp.address, &recipient, 5.0, &kp.signing_private_key, 0, "").await?;
```

## Skills

```rust
let skills = poh.list_skills().await?;
for skill in &skills {
    println!("{} — {:?}", skill.id, skill.description);
}
```

## Bulk scans

```rust
use poh_sdk::PollOptions;

let done = poh.scan_and_wait(
    &["0xaaa", "0xbbb", "0xccc"],
    ScanOptions::default(),
    PollOptions {
        on_progress: Some(|job| println!("{:.0}%", job.percent)),
        ..Default::default()
    },
).await?;

for item in &done.results {
    println!("{} → {:?}", item.input, item.result);
}
```

## Error handling

All methods return `poh_sdk::Result<T>` — `std::result::Result<T, PohError>`.

```rust
use poh_sdk::PohError;

match poh.get_balance("poh...").await {
    Ok(bal)                               => println!("{}", bal.balance),
    Err(PohError::Api { status, message }) => eprintln!("API {status}: {message}"),
    Err(PohError::Network(_))             => eprintln!("network error"),
    Err(e)                                => eprintln!("{e}"),
}
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `tokio` | ✓ | Enables polling helpers (`poll_job`, `scan_and_wait`, `ask_and_wait`) |
| `signing` | — | Ed25519 keypair generation, transaction building and signing |

## License

MIT
