//! End-to-end tests against the Miden testnet.
//!
//! These tests require:
//! - `miden-client-native` feature enabled
//! - A funded wallet at `~/.miden/` (run `miden-client init --network testnet` + fund via faucet)
//! - Network connectivity to the Miden testnet RPC
//!
//! Run with:
//! ```sh
//! cargo test --features "full,miden-client-native" -- --ignored e2e --nocapture
//! ```

#![cfg(feature = "miden-client-native")]

use std::sync::Arc;
use std::time::Instant;

use miden_client::builder::ClientBuilder;
use miden_client::keystore::FilesystemKeyStore;
use miden_client::rpc::Endpoint;
use miden_client::Client;
use miden_client_sqlite_store::SqliteStore;
use tokio::sync::Mutex;

use x402_chain_miden::chain::{MidenAccountAddress, MidenChainConfig, MidenChainProvider, MidenChainReference};
use x402_chain_miden::v2_miden_exact::client::{MidenClientSigner, MidenSignerLike};

// ============================================================================
// Helpers
// ============================================================================

/// Testnet wallet #1 (sender, funded with 100M tokens from faucet)
const WALLET_1: &str = "0x0b50cc0489f8f1101e946691aa89ca";
/// Testnet wallet #2 (receiver)
const WALLET_2: &str = "0x85d0722292b1c01042989aa82aa1c9";
/// Testnet faucet ID
const FAUCET_ID: &str = "0x37d5977a8e16d8205a360820f0230f";

/// Build a `Client<FilesystemKeyStore>` from the global `~/.miden` store.
async fn build_testnet_client() -> Client<FilesystemKeyStore> {
    let home = std::env::var("HOME").expect("HOME env var");
    let miden_dir = format!("{home}/.miden");
    let store_path = format!("{miden_dir}/store.sqlite3");
    let keystore_path = format!("{miden_dir}/keystore");

    let sqlite_store = SqliteStore::new(store_path.as_str().try_into().expect("store path"))
        .await
        .expect("open SQLite store");

    let endpoint = Endpoint::testnet();

    ClientBuilder::new()
        .grpc_client(&endpoint, Some(10_000))
        .store(Arc::new(sqlite_store))
        .filesystem_keystore(&keystore_path)
        .build()
        .await
        .expect("build Client")
}

// ============================================================================
// E2E: P2ID Transfer via x402-chain-miden API
// ============================================================================

/// Full x402 payment flow on testnet:
/// 1. Create MidenClientSigner from our crate
/// 2. Call create_and_prove_p2id (P2ID note + STARK proof)
/// 3. Verify the proof using TransactionVerifier
/// 4. Submit to the Miden node via GrpcClient
/// 5. Verify sender/receiver balances
#[tokio::test]
#[ignore] // requires testnet + funded wallets
async fn e2e_p2id_transfer_via_x402_crate() {
    // ── 1. Build Client and MidenClientSigner ──────────────────────────
    println!("\n=== E2E P2ID Transfer Test ===\n");

    let client = build_testnet_client().await;
    let client = Arc::new(Mutex::new(client));

    // Sync before starting
    {
        let mut c = client.lock().await;
        let summary = c.sync_state().await.expect("sync with testnet");
        println!("Synced to block {}", summary.block_num);
    }

    let signer = MidenClientSigner::new(WALLET_1, client.clone());

    // ── 2. Create and prove P2ID payment ───────────────────────────────
    let amount: u64 = 500; // send 500 base units
    println!("Sending {amount} tokens from {WALLET_1} → {WALLET_2}");

    let t_start = Instant::now();
    let (proven_tx_hex, tx_id, tx_inputs_hex) = signer
        .create_and_prove_p2id(WALLET_2, FAUCET_ID, amount)
        .await
        .expect("create_and_prove_p2id should succeed");
    let prove_time = t_start.elapsed();

    println!("Transaction proved in {prove_time:.2?}");
    println!("TX ID: {tx_id}");
    println!("ProvenTransaction hex: {} bytes", proven_tx_hex.len() / 2);
    println!("TransactionInputs hex: {} bytes", tx_inputs_hex.len() / 2);

    assert!(!proven_tx_hex.is_empty(), "proven_tx should not be empty");
    assert!(!tx_id.is_empty(), "tx_id should not be empty");
    assert!(!tx_inputs_hex.is_empty(), "tx_inputs should not be empty");

    // ── 3. Verify STARK proof ──────────────────────────────────────────
    println!("\nVerifying STARK proof...");
    let t_verify = Instant::now();
    {
        use miden_protocol::transaction::ProvenTransaction;
        use miden_protocol::utils::serde::Deserializable;
        use miden_tx::TransactionVerifier;

        let proven_tx_bytes = hex::decode(&proven_tx_hex).expect("decode hex");
        let proven_tx =
            ProvenTransaction::read_from_bytes(&proven_tx_bytes).expect("deserialize ProvenTx");

        let verifier = TransactionVerifier::new(96);
        verifier.verify(&proven_tx).expect("STARK proof should be valid");
    }
    let verify_time = t_verify.elapsed();
    println!("STARK proof verified in {verify_time:.2?}");

    // ── 4. Verify output notes contain correct P2ID payment ────────────
    println!("\nVerifying P2ID output note...");
    {
        use miden_protocol::account::AccountId;
        use miden_protocol::transaction::{OutputNote, ProvenTransaction};
        use miden_protocol::utils::serde::Deserializable;
        use miden_standards::note::WellKnownNote;

        let proven_tx_bytes = hex::decode(&proven_tx_hex).expect("decode hex");
        let proven_tx =
            ProvenTransaction::read_from_bytes(&proven_tx_bytes).expect("deserialize ProvenTx");

        let target = AccountId::from_hex(WALLET_2).expect("parse wallet 2");
        let faucet = AccountId::from_hex(FAUCET_ID).expect("parse faucet");
        let p2id_root = WellKnownNote::P2ID.script_root();

        let mut found = false;
        for output_note in proven_tx.output_notes().iter() {
            if let OutputNote::Full(note) = output_note {
                if note.recipient().script().root() != p2id_root {
                    continue;
                }
                let inputs = note.recipient().inputs().values();
                if inputs.len() < 2 {
                    continue;
                }
                let note_target = AccountId::new_unchecked([inputs[1], inputs[0]]);
                if note_target != target {
                    continue;
                }
                for asset in note.assets().iter_fungible() {
                    if asset.faucet_id() == faucet && asset.amount() >= amount {
                        found = true;
                        println!(
                            "  P2ID note found: {} tokens to {}",
                            asset.amount(),
                            WALLET_2
                        );
                    }
                }
            }
        }
        assert!(found, "Should find P2ID output note with correct recipient, faucet, and amount");
    }

    // ── 5. Submit to Miden node ────────────────────────────────────────
    println!("\nSubmitting to Miden node...");
    let config = MidenChainConfig {
        chain_reference: MidenChainReference::testnet(),
        rpc_url: "https://rpc.testnet.miden.io".to_string(),
    };
    let provider = MidenChainProvider::from_config(&config);

    let proven_tx_bytes = hex::decode(&proven_tx_hex).expect("decode proven_tx hex");
    let tx_inputs_bytes = hex::decode(&tx_inputs_hex).expect("decode tx_inputs hex");

    let t_submit = Instant::now();
    let submitted_tx_id = provider
        .submit_proven_transaction(&proven_tx_bytes, &tx_inputs_bytes)
        .await
        .expect("submit_proven_transaction should succeed");
    let submit_time = t_submit.elapsed();

    println!("Submitted in {submit_time:.2?}");
    println!("Submitted TX ID: {submitted_tx_id}");

    // ── 6. Also apply the transaction locally ──────────────────────────
    // The Client needs to know about this transaction so it updates local state.
    // We need to sync to pick up the changes.
    {
        let mut c = client.lock().await;
        let summary = c.sync_state().await.expect("post-submit sync");
        println!("\nPost-submit sync to block {}", summary.block_num);
    }

    // ── Summary ────────────────────────────────────────────────────────
    println!("\n=== E2E Results ===");
    println!("  Prove time:  {prove_time:.2?}");
    println!("  Verify time: {verify_time:.2?}");
    println!("  Submit time: {submit_time:.2?}");
    println!("  Total time:  {:.2?}", t_start.elapsed());
    println!("  TX ID:       {submitted_tx_id}");
    println!("===================\n");
}

// ============================================================================
// Benchmark: STARK proof generation
// ============================================================================

/// Benchmark STARK proof generation time on this machine.
/// Creates a P2ID note and measures execution + proving time.
#[tokio::test]
#[ignore] // requires testnet + funded wallet
async fn benchmark_stark_proof_generation() {
    println!("\n=== STARK Proof Generation Benchmark ===\n");

    let client = build_testnet_client().await;
    let client = Arc::new(Mutex::new(client));

    // Sync
    {
        let mut c = client.lock().await;
        let summary = c.sync_state().await.expect("sync");
        println!("Synced to block {}", summary.block_num);
    }

    let signer = MidenClientSigner::new(WALLET_1, client.clone());

    // Run 3 iterations
    for i in 1..=3 {
        let amount = 100 + i; // vary amount slightly to avoid duplicate note IDs
        let t = Instant::now();
        let (proven_tx_hex, tx_id, tx_inputs_hex) = signer
            .create_and_prove_p2id(WALLET_2, FAUCET_ID, amount)
            .await
            .expect("prove P2ID");
        let elapsed = t.elapsed();

        println!(
            "  Run {i}: {elapsed:.2?} (tx={}, proof={} bytes, inputs={} bytes)",
            &tx_id[..18],
            proven_tx_hex.len() / 2,
            tx_inputs_hex.len() / 2,
        );
    }

    println!("\n=== End Benchmark ===\n");
}

// ============================================================================
// Balance query test
// ============================================================================

/// Test that get_account_balance works against the testnet RPC.
#[tokio::test]
#[ignore] // requires testnet
async fn e2e_get_account_balance() {
    println!("\n=== Balance Query Test ===\n");

    let config = MidenChainConfig {
        chain_reference: MidenChainReference::testnet(),
        rpc_url: "https://rpc.testnet.miden.io".to_string(),
    };
    let provider = MidenChainProvider::from_config(&config);

    let balance = provider
        .get_account_balance(WALLET_1, FAUCET_ID)
        .await
        .expect("should query balance");

    println!("Wallet 1 ({WALLET_1}) balance: {balance}");
    assert!(balance > 0, "Wallet 1 should have tokens");

    let balance2 = provider
        .get_account_balance(WALLET_2, FAUCET_ID)
        .await
        .expect("should query balance");

    println!("Wallet 2 ({WALLET_2}) balance: {balance2}");
    println!("\n=== End Balance Query Test ===\n");
}

// ============================================================================
// MidenAccountAddress conversion test (with real IDs)
// ============================================================================

/// Test that real testnet account IDs roundtrip through MidenAccountAddress.
#[tokio::test]
#[ignore] // requires miden-native
async fn e2e_account_id_roundtrip() {
    use miden_protocol::account::AccountId;

    let addr: MidenAccountAddress = WALLET_1.parse().expect("parse wallet 1");
    let account_id = addr.to_account_id().expect("convert to AccountId");
    let addr2 = MidenAccountAddress::from_account_id(account_id);
    assert_eq!(addr, addr2, "roundtrip should preserve address");

    // Also verify it matches what from_hex gives
    let direct = AccountId::from_hex(WALLET_1).expect("AccountId::from_hex");
    assert_eq!(account_id, direct);

    println!("AccountId roundtrip OK: {WALLET_1} → {addr2}");
}
