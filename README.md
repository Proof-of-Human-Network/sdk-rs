# poh-sdk

Rust SDK for the [Decentralized Artificial Intelligence](https://iamai.kg) network.

## Add to your project

```toml
[dependencies]
poh-sdk = "0.5"
tokio   = { version = "1", features = ["full"] }

# Enable signing utilities:
poh-sdk = { version = "0.5", features = ["signing"] }
```

## Quick start

```rust
use poh_sdk::{DAIClient, DAIClientOptions, ScanOptions};

#[tokio::main]
async fn main() -> poh_sdk::Result<()> {
    let dai = DAIClient::new(
        DAIClientOptions::new("https://miner.iamai.kg")
            .api_key("your-api-key"),
    );

    let res = dai.scan("0xabc...", ScanOptions::default()).await?;
    match res.result {
        Some(true)  => println!("Human"),
        Some(false) => println!("Bot"),
        None        => println!("Inconclusive"),
    }
    Ok(())
}
```

## Multi-node failover

The client can probe a list of nodes and use the first one that responds.
`DAIClientOptions::default()` uses the built-in default node list.

```rust
let dai = DAIClient::new(DAIClientOptions::with_nodes([
    "https://miner.iamai.kg",
    "https://iamai.kg",
    "https://miner.iamai.kg",
]));

// Which node was selected (None before the first request)?
println!("{:?}", dai.active_node());
```

## Local miner routing

Write operations (any non-GET request except `POST /gossip`) must go to a node
you control. Set `local_base_url` to route them to your local miner while reads
still hit the public nodes; without it, writes to a non-loopback node fail with
a 403 explaining the requirement.

```rust
let dai = DAIClient::new(
    DAIClientOptions::default().local_base_url("http://127.0.0.1:3456"),
);
```

## Brain verdict

`scan` returns raw method results; the aggregated AI verdict is fetched
separately by brain key.

```rust
// Fetch / poll by brain key
let verdict = dai.get_brain_verdict("brain-key").await?;
let verdict = dai.poll_brain_verdict("brain-key", Default::default()).await?; // BrainPollOptions

// Scan + verdict in one call
let sv = dai.scan_and_verdict("0xabc...", ScanOptions::default(), Default::default()).await?;
println!("{:?} {:?}", sv.verdict.verdict, sv.verdict.confidence);
```

## Verification methods

```rust
let methods = dai.get_methods(None).await?;          // or Some("dai...") for wallet-specific
let method  = dai.get_method("method-id").await?;
```

## Natural language jobs

Skill jobs always require a fee — set `budget`, `wallet_address`, and
`private_key_pem` on `AskOptions` (requires the `signing` feature) so the SDK
can sign the payment. The node verifies the signature and debits the fee
before it will run the job at all; it rejects the request outright (no job
ever runs) without a valid signed payment.

```rust
use poh_sdk::{DAIClient, DAIClientOptions, AskOptions};

let dai = DAIClient::new(DAIClientOptions::new("https://iamai.kg"));

let result = dai.ask_and_wait(
    "What does vitalik.eth write about on Paragraph?",
    AskOptions::new(0.5).wallet("dai...").private_key(my_private_key_pem),
    Default::default(),
).await?;

println!("{:?}", result.output);
if let Some(nl) = result.nl_response { println!("{nl}"); }
```

Or fire-and-poll manually:

```rust
let job_ref = dai.submit_job("...", AskOptions::new(0.5).wallet("dai...").private_key(pem)).await?;
let status  = dai.get_job_status(&job_ref.job_id).await?;   // lightweight status check
let result  = dai.get_job_result(&job_ref.job_id).await?;   // full result once done
let result  = dai.poll_job_result(&job_ref.job_id, Default::default()).await?; // poll until done
```

## Compute jobs (your own model + dataset)

Run inference with a model of your choice, optionally grounded in a Hugging
Face dataset already installed on the node. Like skill jobs, compute jobs are
never free — `run_compute` always signs a fee payment. Requires the `signing`
feature.

```rust
use poh_sdk::{DAIClient, DAIClientOptions, ComputeOptions};

let dai = DAIClient::new(DAIClientOptions::new("https://iamai.kg"));

let opts = ComputeOptions::new("llama3.1:8b", 0.5, "dai...", my_private_key_pem)
    .dataset("some-org/some-dataset"); // optional

let job_ref = dai.run_compute("Summarize the top 5 rows", opts).await?;
let result  = dai.poll_job_result(&job_ref.job_id, Default::default()).await?;
println!("{:?}", result.output);
```

Before either of these will work, the wallet's signing key must be registered
with the node once via `register_signing_key()` — the node has no way to
verify a signature for a key it has never seen.

## Wallet / blockchain

```rust
// Balance (μDAI — divide by 1e9 for DAI)
let bal = dai.get_balance("dai...").await?;
println!("{} μDAI", bal.balance);

// Nonce (needed before building a transaction)
let nonce = dai.get_nonce("dai...").await?;

// Transaction history
let history = dai.get_transaction_history("dai...", 50).await?;
for entry in &history.entries {
    println!("{}: {} μDAI", entry.tx_hash, entry.delta);
}

// Raw transactions for an address (untyped JSON)
let txs = dai.get_transactions("dai...").await?;

// Pending mempool transactions
let pending = dai.get_pending_transactions().await?;
println!("{} pending txs", pending.count);

// Miner info
let info = dai.get_miner_info().await?;
println!("{} reputation={}", info.model, info.reputation);

// Basic node health / metadata
let node = dai.get_node_info().await?;
```

## Signing & transactions

Requires the `signing` feature:

```toml
poh-sdk = { version = "0.5", features = ["signing"] }
```

```rust
use poh_sdk::{generate_key_pair, build_transfer, sign_transaction, create_signing_proof};

// 1. Generate a keypair — address is derived from the signing public key
let kp = generate_key_pair()?;

// 2. Register with your local node (one-time, per node)
dai.register_key_pair(&kp, None).await?;

// 3. Build, sign, and submit a transfer
let nonce_resp = dai.get_nonce(&kp.address).await?;
let tx     = build_transfer(&kp.address, &recipient, 5.0, nonce_resp.nonce + 1, 0, "")?;
let signed = sign_transaction(&tx, &kp.signing_private_key)?;
let result = dai.submit_transaction(&signed).await?;
println!("{}", result.tx_hash);

// One-liner convenience (fetches nonce automatically)
let result = dai.transfer(&kp.address, &recipient, 5.0, &kp.signing_private_key, 0, "").await?;
```

### Signing helpers

```rust
use poh_sdk::{
    derive_address_from_signing_key, sign_data, create_rotation_proof,
    compute_tx_hash, compute_tx_hash_with_currency,
    compute_job_payment_hash, sign_job_payment, generate_job_id, decimals_of,
};

// Address derived from an SPKI PEM signing public key
let addr = derive_address_from_signing_key(&kp.signing_public_key);

// Sign an arbitrary UTF-8 message (base64 Ed25519 signature)
let sig = sign_data("hello", &kp.signing_private_key)?;

// Rotation proof — replace an already-registered key (signed with the OLD key)
let proof = create_rotation_proof(&addr, &new_kp.signing_public_key, &old_private_key_pem)?;
dai.register_key_pair(&new_kp, Some(&proof)).await?;

// Canonical SHA-256 tx hash (currency-aware variant appends `currency` only when non-DAI)
let hash = compute_tx_hash(&from, &to, 5_000_000_000, 0, 42, timestamp_ms, "");
let hash = compute_tx_hash_with_currency(&from, &to, 1250, 0, 42, timestamp_ms, "", Some("aiGEL"));

// Job fee payment — hash binds the fee to one job + miner + amount + nonce
let job_id = generate_job_id();          // "job-<millis>-<8 hex>"; fixed before signing
let hash   = compute_job_payment_hash(&job_id, &me, &miner, 500, nonce);
let (tx_hash, signature) = sign_job_payment(&job_id, &me, &miner, 500, nonce, &pem)?;
// (used internally by submit_job / run_compute)

// Decimals for an on-chain asset (9 for DAI, 2 for the stablecoins)
assert_eq!(decimals_of(Some("aiGEL")), 2);
```

## Skills

```rust
let skills = dai.list_skills().await?;
for skill in &skills {
    println!("{} — {:?}", skill.id, skill.description);
}
```

## Bulk scans

```rust
use poh_sdk::PollOptions;

let done = dai.scan_and_wait(
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

## Chat crypto

End-to-end encryption for chat payloads (X25519 + HKDF + AES-256-GCM),
compatible with the node's envelope format. Requires the `chatcrypto` feature:

```toml
poh-sdk = { version = "0.5", features = ["chatcrypto"] }
```

```rust
use poh_sdk::{derive_encryption_keypair, seal, open};

// Deterministic X25519 keypair from a stable secret (e.g. the signing key)
let kp = derive_encryption_keypair(stable_secret_bytes);

// Encrypt for a recipient / decrypt an envelope
let env       = seal(&recipient.public_key_b64, b"hello")?;   // SealedEnvelope
let plaintext = open(&env, &kp.private_key_b64)?;
```

## Error handling

All methods return `poh_sdk::Result<T>` — `std::result::Result<T, DAIError>`.

```rust
use poh_sdk::DAIError;

match dai.get_balance("dai...").await {
    Ok(bal)                               => println!("{}", bal.balance),
    Err(DAIError::Api { status, message }) => eprintln!("API {status}: {message}"),
    Err(DAIError::Network(_))             => eprintln!("network error"),
    Err(e)                                => eprintln!("{e}"),
}
```

## Features

| Feature | Default | Description |
|---------|---------|-------------|
| `tokio` | ✓ | Enables polling helpers (`poll_job`, `scan_and_wait`, `ask_and_wait`) and node discovery |
| `signing` | — | Ed25519 keypair generation, transaction building and signing |
| `chatcrypto` | — | X25519/HKDF/AES-GCM chat envelope encryption (`derive_encryption_keypair`, `seal`, `open`) |

## License

MIT

## Stablecoins (multi-currency)

Five regional stablecoins ride alongside DAI: `aiGEL`, `aiKGS`, `aiAMD`,
`aiETB`, `aiBTN` (2 decimals; DAI keeps 9 — μDAI).

```rust
// Transfer 12.50 aiGEL (display units; scaled at the asset's own decimals)
let tx = build_transfer_with_currency(&from, &to, 12.5, nonce + 1, 0, "", "aiGEL")?;
let signed = sign_transaction(&tx, &private_key_pem)?;

// Job payment in a stablecoin (6th-key rule — DAI hashes unchanged)
let hash = compute_job_payment_hash_with_currency(&job_id, &me, &miner, 500, nonce, Some("aiKGS"));
```

DAI transactions hash byte-identically to the historical preimage (`currency`
enters the signed payload only when non-DAI) — existing integrations are
unaffected.
