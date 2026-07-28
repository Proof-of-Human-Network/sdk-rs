//! Integration tests for PohClient — all public methods.
//! Uses wiremock to intercept HTTP calls; no live server required.

use poh_sdk::{
    AskOptions, BrainPollOptions, ComputeOptions, PohClient, PohClientOptions, PollOptions,
    ScanOptions, JobStatusCode, generate_key_pair,
};
use serde_json::json;
use wiremock::{
    matchers::{method, path, path_regex},
    Mock, MockServer, ResponseTemplate,
};

fn client(uri: &str) -> PohClient {
    PohClient::new(PohClientOptions::new(uri))
}

// ── scan ──────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scan_returns_result_and_brain_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/checker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": true, "brainKey": "bk-1", "freeScansLeft": 9
        })))
        .mount(&server)
        .await;

    let res = client(&server.uri())
        .scan("0xabc", ScanOptions::default())
        .await
        .unwrap();
    assert_eq!(res.result, Some(true));
    assert_eq!(res.brain_key.as_deref(), Some("bk-1"));
    assert_eq!(res.free_scans_left, Some(9));
}

#[tokio::test]
async fn scan_returns_none_result_for_inconclusive() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/checker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "result": null, "freeScansLeft": 8
        })))
        .mount(&server)
        .await;

    let res = client(&server.uri())
        .scan("0xabc", ScanOptions::default())
        .await
        .unwrap();
    assert_eq!(res.result, None);
}

#[tokio::test]
async fn scan_returns_error_on_4xx() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/checker"))
        .respond_with(ResponseTemplate::new(403).set_body_json(json!({"error": "forbidden"})))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .scan("0xabc", ScanOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, poh_sdk::PohError::Api { status: 403, .. }));
}

// ── scan_bulk ─────────────────────────────────────────────────────────────────

#[tokio::test]
async fn scan_bulk_returns_job_id_and_total() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/checker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "j-1", "status": "queued", "total": 2,
            "pollUrl": "/checker/job/j-1", "freeScansLeft": 5
        })))
        .mount(&server)
        .await;

    let res = client(&server.uri())
        .scan_bulk(&["0xaaa", "0xbbb"], ScanOptions::default())
        .await
        .unwrap();
    assert_eq!(res.job_id, "j-1");
    assert_eq!(res.total, 2);
    assert_eq!(res.status, JobStatusCode::Queued);
}

#[tokio::test]
async fn scan_bulk_rejects_empty_slice() {
    let err = client("http://unused")
        .scan_bulk(&[], ScanOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, poh_sdk::PohError::InvalidArgument(_)));
}

// ── get_job ───────────────────────────────────────────────────────────────────

#[tokio::test]
async fn get_job_parses_snapshot() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/checker/job/j-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "j-1", "status": "done", "total": 2, "done": 2, "percent": 100.0,
            "results": [{"input": "0xaaa", "result": true, "error": null}],
            "errors": [], "createdAt": "2024-01-01T00:00:00Z", "completedAt": null
        })))
        .mount(&server)
        .await;

    let job = client(&server.uri()).get_job("j-1").await.unwrap();
    assert!(job.is_terminal());
    assert_eq!(job.percent, 100.0);
    assert_eq!(job.results.len(), 1);
    assert_eq!(job.results[0].result, Some(true));
}

// ── poll_job ──────────────────────────────────────────────────────────────────

#[tokio::test]
async fn poll_job_terminates_when_server_returns_done() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/checker/job/j-done"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "j-done", "status": "done", "total": 1, "done": 1, "percent": 100.0,
            "results": [], "errors": [], "createdAt": "2024-01-01T00:00:00Z", "completedAt": null
        })))
        .mount(&server)
        .await;

    let job = client(&server.uri())
        .poll_job("j-done", PollOptions::default())
        .await
        .unwrap();
    assert_eq!(job.status, JobStatusCode::Done);
}

#[tokio::test]
async fn poll_job_timeout_returns_poll_timeout_error() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/checker/job/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "j-processing", "status": "processing", "total": 1, "done": 0, "percent": 0.0,
            "results": [], "errors": [], "createdAt": "2024-01-01T00:00:00Z", "completedAt": null
        })))
        .mount(&server)
        .await;

    let opts = PollOptions {
        interval: std::time::Duration::from_millis(10),
        timeout:  std::time::Duration::from_millis(20),
        on_progress: None,
    };
    let err = client(&server.uri())
        .poll_job("j-processing", opts)
        .await
        .unwrap_err();
    assert!(matches!(err, poh_sdk::PohError::PollTimeout));
}

// ── scan_and_wait ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn scan_and_wait_combines_bulk_and_poll() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/checker"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "j-sw", "status": "queued", "total": 1,
            "pollUrl": "/checker/job/j-sw", "freeScansLeft": 3
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/checker/job/j-sw"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "j-sw", "status": "done", "total": 1, "done": 1, "percent": 100.0,
            "results": [], "errors": [], "createdAt": "2024-01-01T00:00:00Z", "completedAt": null
        })))
        .mount(&server)
        .await;

    let job = client(&server.uri())
        .scan_and_wait(&["0xabc"], ScanOptions::default(), PollOptions::default())
        .await
        .unwrap();
    assert_eq!(job.status, JobStatusCode::Done);
}

// ── get_brain_verdict ─────────────────────────────────────────────────────────

#[tokio::test]
async fn get_brain_verdict_parses_done() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/checker/brain/bk-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "done", "verdict": "HUMAN", "confidence": 0.91, "reasoning": "active"
        })))
        .mount(&server)
        .await;

    let v = client(&server.uri()).get_brain_verdict("bk-1").await.unwrap();
    assert_eq!(v.status, "done");
    assert_eq!(v.verdict.as_deref(), Some("HUMAN"));
    assert_eq!(v.confidence, Some(0.91));
}

#[tokio::test]
async fn get_brain_verdict_parses_pending() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/checker/brain/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"status": "pending"})))
        .mount(&server)
        .await;

    let v = client(&server.uri()).get_brain_verdict("bk-2").await.unwrap();
    assert_eq!(v.status, "pending");
}

// ── poll_brain_verdict ────────────────────────────────────────────────────────

#[tokio::test]
async fn poll_brain_verdict_returns_when_not_pending() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/checker/brain/.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "done", "verdict": "AI", "confidence": 0.7
        })))
        .mount(&server)
        .await;

    let v = client(&server.uri())
        .poll_brain_verdict("bk-x", BrainPollOptions::default())
        .await
        .unwrap();
    assert_eq!(v.verdict.as_deref(), Some("AI"));
}

// ── get_methods / get_method ──────────────────────────────────────────────────

#[tokio::test]
async fn get_methods_returns_method_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/verifyer"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "m1", "type": "evm", "description": "ETH balance", "score": 1.0, "voteCount": 5}
        ])))
        .mount(&server)
        .await;

    let methods = client(&server.uri()).get_methods(None).await.unwrap();
    assert_eq!(methods.len(), 1);
    assert_eq!(methods[0].id, "m1");
    assert_eq!(methods[0].method_type, "evm");
}

#[tokio::test]
async fn get_method_returns_single_by_id() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/verifyer/m2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "m2", "type": "solana", "description": "SOL staking", "score": 2.5
        })))
        .mount(&server)
        .await;

    let m = client(&server.uri()).get_method("m2").await.unwrap();
    assert_eq!(m.id, "m2");
    assert_eq!(m.method_type, "solana");
}

// ── submit_job ────────────────────────────────────────────────────────────────

#[tokio::test]
async fn submit_job_routes_to_skill_then_submits() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/route"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "skill", "skillId": "sk-sum", "input": {}
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/job"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jnl-1", "status": "queued"
        })))
        .mount(&server)
        .await;

    // budget=0 (free job) — no signed payment required.
    let job_ref = client(&server.uri())
        .submit_job("Summarise this", AskOptions::new(0.0))
        .await
        .unwrap();
    assert_eq!(job_ref.job_id, "jnl-1");
}

#[tokio::test]
async fn submit_job_errors_when_budget_positive_without_private_key() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/route"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "skill", "skillId": "sk-sum", "input": {}
        })))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .submit_job("Summarise this", AskOptions::new(0.5).wallet("pohAlice"))
        .await
        .unwrap_err();
    assert!(matches!(err, poh_sdk::PohError::InvalidArgument(_)));
}

#[tokio::test]
async fn submit_job_signs_a_nonce_bound_payment_proof_when_budget_positive() {
    let server = MockServer::start().await;
    let kp = generate_key_pair().unwrap();

    Mock::given(method("POST"))
        .and(path("/chat/route"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "skill", "skillId": "sk-sum", "input": {}
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/miner/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "minerAddress": "pohMiner", "gasPrice": 1, "model": "qwen2.5:1.5b",
            "queueLength": 0, "reputation": 1.0,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/wallet/nonce"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "address": "pohAlice", "nonce": 3,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/job"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jnl-1", "status": "queued"
        })))
        .mount(&server)
        .await;

    let job_ref = client(&server.uri())
        .submit_job("Summarise this", AskOptions::new(0.5).wallet("pohAlice").private_key(&kp.signing_private_key))
        .await
        .unwrap();
    assert_eq!(job_ref.job_id, "jnl-1");
}

// ── run_compute ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn run_compute_errors_when_budget_not_positive() {
    let server = MockServer::start().await;
    let kp = generate_key_pair().unwrap();
    let err = client(&server.uri())
        .run_compute("hi", ComputeOptions::new("qwen2.5:1.5b", 0.0, "pohAlice", &kp.signing_private_key))
        .await
        .unwrap_err();
    assert!(matches!(err, poh_sdk::PohError::InvalidArgument(_)));
}

#[tokio::test]
async fn run_compute_signs_payment_and_posts_model_dataset() {
    let server = MockServer::start().await;
    let kp = generate_key_pair().unwrap();

    Mock::given(method("GET"))
        .and(path("/api/miner/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "minerAddress": "pohMiner", "gasPrice": 1, "model": "qwen2.5:1.5b",
            "queueLength": 0, "reputation": 1.0,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/wallet/nonce"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "address": "pohAlice", "nonce": 7,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/job"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jc-1", "status": "queued"
        })))
        .mount(&server)
        .await;

    let opts = ComputeOptions::new("llama3.1:8b", 0.5, "pohAlice", &kp.signing_private_key)
        .dataset("some-org/some-dataset");
    let job_ref = client(&server.uri())
        .run_compute("Summarize the top rows", opts)
        .await
        .unwrap();
    assert_eq!(job_ref.job_id, "jc-1");
}

#[tokio::test]
async fn submit_job_returns_error_when_no_skill_matched() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/chat/route"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "type": "chat", "reason": "No skill matched"
        })))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .submit_job("random question", AskOptions::default())
        .await
        .unwrap_err();
    assert!(matches!(err, poh_sdk::PohError::InvalidArgument(_)));
}

// ── get_job_status / get_job_result ───────────────────────────────────────────

#[tokio::test]
async fn get_job_status_returns_status_for_nl_job() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/job/jnl-1/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jnl-1", "status": "computing"
        })))
        .mount(&server)
        .await;

    let s = client(&server.uri()).get_job_status("jnl-1").await.unwrap();
    assert_eq!(s.status, "computing");
}

#[tokio::test]
async fn get_job_result_parses_completed_result() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/job/jnl-1/result"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jnl-1",
            "profile": {
                "skillOutput": {"answer": 42},
                "skillId": "sk-1",
                "tokensUsed": 10,
                "nlResponse": "The answer is 42."
            }
        })))
        .mount(&server)
        .await;

    let r = client(&server.uri()).get_job_result("jnl-1").await.unwrap();
    assert_eq!(r.job_id, "jnl-1");
    assert_eq!(r.nl_response.as_deref(), Some("The answer is 42."));
    assert_eq!(r.tokens_used, Some(10));
    assert_eq!(r.skill_id.as_deref(), Some("sk-1"));
}

// ── poll_job_result / ask_and_wait ────────────────────────────────────────────

#[tokio::test]
async fn poll_job_result_fetches_result_when_done() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/job/jnl-2/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jobId":"jnl-2","status":"done"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/job/jnl-2/result"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jnl-2",
            "profile": {"nlResponse": "Done!", "skillId": "sk-1", "tokensUsed": 5, "skillOutput": null}
        })))
        .mount(&server)
        .await;

    let r = client(&server.uri())
        .poll_job_result("jnl-2", PollOptions::default())
        .await
        .unwrap();
    assert_eq!(r.nl_response.as_deref(), Some("Done!"));
}

#[tokio::test]
async fn ask_and_wait_routes_submits_and_polls() {
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/chat/route"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"type":"skill","skillId":"sk-1","input":{}})))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/job"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jobId":"jnl-3","status":"queued"})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/job/jnl-3/status"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"jobId":"jnl-3","status":"done"})))
        .mount(&server).await;
    Mock::given(method("GET")).and(path("/job/jnl-3/result"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "jobId": "jnl-3",
            "profile": {"nlResponse": "Answer", "skillId": "sk-1", "tokensUsed": 8, "skillOutput": null}
        })))
        .mount(&server).await;

    let r = client(&server.uri())
        .ask_and_wait("What is 2+2?", AskOptions::default(), PollOptions::default())
        .await
        .unwrap();
    assert_eq!(r.nl_response.as_deref(), Some("Answer"));
}

// ── get_node_info / list_skills / get_miner_info ──────────────────────────────

#[tokio::test]
async fn get_node_info_returns_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/healthz"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "status": "ok", "nodeId": "node-42", "version": "1.2.0", "peers": 3
        })))
        .mount(&server)
        .await;

    let info = client(&server.uri()).get_node_info().await.unwrap();
    assert_eq!(info.node_id.as_deref(), Some("node-42"));
    assert_eq!(info.version.as_deref(), Some("1.2.0"));
    assert_eq!(info.peers, Some(3));
}

#[tokio::test]
async fn list_skills_returns_skill_array() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/skills"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([
            {"id": "sk-1", "description": "Summariser", "triggers": ["summarise"]}
        ])))
        .mount(&server)
        .await;

    let skills = client(&server.uri()).list_skills().await.unwrap();
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].id, "sk-1");
}

#[tokio::test]
async fn get_miner_info_returns_miner_metadata() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/miner/info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "minerAddress": "poh-miner-1", "gasPrice": 1000, "model": "llama-3",
            "queueLength": 2, "reputation": 4.5
        })))
        .mount(&server)
        .await;

    let info = client(&server.uri()).get_miner_info().await.unwrap();
    assert_eq!(info.miner_address, "poh-miner-1");
    assert_eq!(info.model, "llama-3");
}

// ── wallet / blockchain ───────────────────────────────────────────────────────

#[tokio::test]
async fn get_balance_returns_address_and_μpoh() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/api/wallet/balance.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "address": "poh123", "balance": 5_000_000_000i64
        })))
        .mount(&server)
        .await;

    let bal = client(&server.uri()).get_balance("poh123").await.unwrap();
    assert_eq!(bal.address, "poh123");
    assert_eq!(bal.balance, 5_000_000_000);
}

#[tokio::test]
async fn get_nonce_returns_current_nonce() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/api/wallet/nonce.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "address": "poh123", "nonce": 7i64
        })))
        .mount(&server)
        .await;

    let n = client(&server.uri()).get_nonce("poh123").await.unwrap();
    assert_eq!(n.nonce, 7);
}

#[tokio::test]
async fn get_transaction_history_returns_entries() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path_regex("/api/wallet/history.*"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "address": "poh123",
            "entries": [{"height": 100i64, "delta": 1_000_000_000i64, "txHash": "abc", "ts": 1700000000i64, "label": "transfer"}]
        })))
        .mount(&server)
        .await;

    let hist = client(&server.uri())
        .get_transaction_history("poh123", 30)
        .await
        .unwrap();
    assert_eq!(hist.address, "poh123");
    assert_eq!(hist.entries.len(), 1);
    assert_eq!(hist.entries[0].delta, 1_000_000_000);
    assert_eq!(hist.entries[0].label, "transfer");
}

#[tokio::test]
async fn get_pending_transactions_returns_queue() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/tx/pending"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"txs": [], "count": 0i64})))
        .mount(&server)
        .await;

    let p = client(&server.uri()).get_pending_transactions().await.unwrap();
    assert_eq!(p.count, 0);
    assert!(p.txs.is_empty());
}

#[tokio::test]
async fn submit_transaction_posts_tx_and_returns_hash() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/tx/submit"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "txHash": "cafebabe", "queueSize": 1i64
        })))
        .mount(&server)
        .await;

    let tx = poh_sdk::PohTx {
        from: "pohA".into(), to: "pohB".into(),
        amount: 1_000_000_000, fee: 0, nonce: 1, timestamp: 1700000000000, memo: "".into(),
        currency: None,
        tx_hash: Some("cafebabe".into()), signature: Some("sig".into()),
        signing_public_key: Some("pub".into()),
    };
    let r = client(&server.uri()).submit_transaction(&tx).await.unwrap();
    assert_eq!(r.tx_hash, "cafebabe");
    assert!(r.ok);
}

#[tokio::test]
async fn register_signing_key_posts_and_succeeds() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/wallet/register-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .mount(&server)
        .await;

    let res = client(&server.uri())
        .register_signing_key("pohA", "pubkey-pem", "proof-b64", None)
        .await
        .unwrap();
    assert_eq!(res["success"], json!(true));
}
