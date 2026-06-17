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

```rust
use poh_sdk::{PohClient, PohClientOptions, AskOptions};

let poh = PohClient::new(PohClientOptions::new("https://proofofhuman.ge"));

let result = poh.ask_and_wait(
    "What does vitalik.eth write about on Paragraph?",
    AskOptions { budget: 0.5, wallet_address: Some("poh...".into()), ..Default::default() },
    Default::default(),
).await?;

println!("{:?}", result.output);
if let Some(nl) = result.nl_response { println!("{nl}"); }
```

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

// 1. Generate a keypair
let kp = generate_key_pair()?;

// 2. Register the public key with the node (one-time, per node)
let proof = create_signing_proof(&my_address, &kp.signing_private_key)?;
poh.register_signing_key(&my_address, &kp.signing_public_key, &proof).await?;

// 3. Build, sign, and submit a transfer
let nonce_resp = poh.get_nonce(&my_address).await?;
let tx     = build_transfer(&my_address, &recipient, 5.0, nonce_resp.nonce + 1, 0, "")?;
let signed = sign_transaction(&tx, &kp.signing_private_key)?;
let result = poh.submit_transaction(&signed).await?;
println!("{}", result.tx_hash);

// One-liner convenience (fetches nonce automatically)
let result = poh.transfer(&my_address, &recipient, 5.0, &kp.signing_private_key, 0, "").await?;
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
