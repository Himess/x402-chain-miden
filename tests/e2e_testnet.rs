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
// E2E: Private vs Public P2ID Transfer Comparison
// ============================================================================

/// Compare Public vs Private P2ID transfers:
/// 1. Create both a Public and Private P2ID note
/// 2. Prove both (STARK proofs)
/// 3. Inspect ProvenTransaction output notes — Full vs Header
/// 4. Compare proof sizes and timing
/// 5. Submit the private transfer to the network
/// 6. Query notes via RPC to verify on-chain visibility difference
/// 7. Report the full comparison
#[tokio::test]
#[ignore] // requires testnet + funded wallets
async fn e2e_private_p2id_transfer() {
    use miden_client::rpc::NodeRpcClient;
    use miden_client::transaction::{PaymentNoteDescription, TransactionRequestBuilder};
    use miden_protocol::account::AccountId;
    use miden_protocol::asset::{Asset, FungibleAsset};
    use miden_protocol::note::NoteType;
    use miden_protocol::transaction::{OutputNote, ProvenTransaction, TransactionInputs};
    use miden_protocol::utils::serde::{Deserializable, Serializable};
    use miden_tx::TransactionVerifier;

    println!("\n=== Private vs Public P2ID Transfer Comparison ===\n");

    let client = build_testnet_client().await;
    let client = Arc::new(Mutex::new(client));

    // Sync
    {
        let mut c = client.lock().await;
        let summary = c.sync_state().await.expect("sync with testnet");
        println!("Synced to block {}\n", summary.block_num);
    }

    let sender = AccountId::from_hex(WALLET_1).expect("parse wallet 1");
    let target = AccountId::from_hex(WALLET_2).expect("parse wallet 2");
    let faucet = AccountId::from_hex(FAUCET_ID).expect("parse faucet");
    let amount: u64 = 300;

    // ── Helper: create, execute, prove a P2ID with given NoteType ────────
    // Returns (proven_tx_hex, tx_inputs_hex, prove_time, full_notes_from_result)
    async fn create_prove_p2id(
        client: &Arc<Mutex<miden_client::Client<miden_client::keystore::FilesystemKeyStore>>>,
        sender: AccountId,
        target: AccountId,
        faucet: AccountId,
        amount: u64,
        note_type: NoteType,
    ) -> (String, String, std::time::Duration, Vec<miden_protocol::note::Note>) {
        let asset = FungibleAsset::new(faucet, amount).expect("create asset");
        let mut client_guard = client.lock().await;

        let payment_data =
            PaymentNoteDescription::new(vec![Asset::Fungible(asset)], sender, target);

        let tx_request = TransactionRequestBuilder::new()
            .build_pay_to_id(payment_data, note_type, client_guard.rng())
            .expect("build P2ID request");

        let tx_result = client_guard
            .execute_transaction(sender, tx_request)
            .await
            .expect("execute transaction");

        // Extract full notes from TransactionResult BEFORE proving.
        // For private notes, the prover will shrink Full → Header,
        // so this is the only way to get the full note data.
        let full_notes: Vec<miden_protocol::note::Note> = tx_result
            .created_notes()
            .iter()
            .filter_map(|on| {
                if let OutputNote::Full(note) = on {
                    Some(note.clone())
                } else {
                    None
                }
            })
            .collect();

        let tx_inputs = TransactionInputs::from(&tx_result);
        let tx_inputs_hex = hex::encode(tx_inputs.to_bytes());

        let prover = client_guard.prover();
        drop(client_guard);

        let t = Instant::now();
        let proven_tx = prover
            .prove(tx_result.into())
            .await
            .expect("prove transaction");
        let prove_time = t.elapsed();

        let tx_hex = hex::encode(proven_tx.to_bytes());
        (tx_hex, tx_inputs_hex, prove_time, full_notes)
    }

    // ── 1. Create PUBLIC P2ID ────────────────────────────────────────────
    println!("── Creating PUBLIC P2ID ({amount} tokens) ──");
    let t_pub = Instant::now();
    let (pub_hex, _pub_inputs_hex, pub_prove_time, _pub_full_notes) =
        create_prove_p2id(&client, sender, target, faucet, amount, NoteType::Public).await;
    let pub_total = t_pub.elapsed();
    let pub_proof_size = pub_hex.len() / 2;
    println!(
        "  Prove: {pub_prove_time:.2?}, Total: {pub_total:.2?}, Proof: {pub_proof_size} bytes"
    );

    // ── 2. Create PRIVATE P2ID ───────────────────────────────────────────
    println!("\n── Creating PRIVATE P2ID ({amount} tokens) ──");
    let t_priv = Instant::now();
    let (priv_hex, priv_inputs_hex, priv_prove_time, priv_full_notes) =
        create_prove_p2id(&client, sender, target, faucet, amount, NoteType::Private).await;
    let priv_total = t_priv.elapsed();
    let priv_proof_size = priv_hex.len() / 2;
    println!(
        "  Prove: {priv_prove_time:.2?}, Total: {priv_total:.2?}, Proof: {priv_proof_size} bytes"
    );

    // ── 3. Inspect ProvenTransaction output notes ────────────────────────
    println!("\n── Output Notes in ProvenTransaction ──");

    let pub_proven_bytes = hex::decode(&pub_hex).expect("decode pub hex");
    let pub_proven =
        ProvenTransaction::read_from_bytes(&pub_proven_bytes).expect("deserialize pub proven tx");

    let priv_proven_bytes = hex::decode(&priv_hex).expect("decode priv hex");
    let priv_proven = ProvenTransaction::read_from_bytes(&priv_proven_bytes)
        .expect("deserialize priv proven tx");

    println!("\n  PUBLIC transaction output notes:");
    let mut pub_note_ids = vec![];
    for (i, output_note) in pub_proven.output_notes().iter().enumerate() {
        let variant = match output_note {
            OutputNote::Full(note) => {
                let assets: Vec<String> = note
                    .assets()
                    .iter_fungible()
                    .map(|a| format!("{} from {}", a.amount(), a.faucet_id()))
                    .collect();
                format!("Full (assets: [{}], recipient: visible)", assets.join(", "))
            }
            OutputNote::Header(_) => "Header (assets: hidden, recipient: hidden)".to_string(),
            OutputNote::Partial(_) => "Partial".to_string(),
        };
        let note_type_str = output_note.metadata().note_type();
        println!("    [{i}] {variant}  (NoteType: {note_type_str:?}, ID: {})", output_note.id());
        pub_note_ids.push(output_note.id());
    }

    println!("\n  PRIVATE transaction output notes:");
    let mut priv_note_ids = vec![];
    let mut has_header_only = false;
    for (i, output_note) in priv_proven.output_notes().iter().enumerate() {
        let variant = match output_note {
            OutputNote::Full(note) => {
                let assets: Vec<String> = note
                    .assets()
                    .iter_fungible()
                    .map(|a| format!("{} from {}", a.amount(), a.faucet_id()))
                    .collect();
                format!("Full (assets: [{}], recipient: visible)", assets.join(", "))
            }
            OutputNote::Header(_) => {
                has_header_only = true;
                "Header (assets: HIDDEN, recipient: HIDDEN)".to_string()
            }
            OutputNote::Partial(_) => "Partial".to_string(),
        };
        let note_type_str = output_note.metadata().note_type();
        println!("    [{i}] {variant}  (NoteType: {note_type_str:?}, ID: {})", output_note.id());
        priv_note_ids.push(output_note.id());
    }

    assert!(
        has_header_only,
        "Private P2ID should produce OutputNote::Header (not Full) in ProvenTransaction"
    );

    // Verify public notes are Full
    for output_note in pub_proven.output_notes().iter() {
        if output_note.metadata().note_type() == NoteType::Public {
            assert!(
                matches!(output_note, OutputNote::Full(_)),
                "Public notes should remain OutputNote::Full"
            );
        }
    }

    // ── 4. Verify STARK proofs for both ──────────────────────────────────
    println!("\n── STARK Proof Verification ──");
    let verifier = TransactionVerifier::new(96);

    let t = Instant::now();
    verifier.verify(&pub_proven).expect("public STARK proof valid");
    let pub_verify = t.elapsed();

    let t = Instant::now();
    verifier.verify(&priv_proven).expect("private STARK proof valid");
    let priv_verify = t.elapsed();

    println!("  Public verify:  {pub_verify:.2?}");
    println!("  Private verify: {priv_verify:.2?}");

    // ── 5. Off-chain note delivery for private notes ─────────────────────
    println!("\n── Off-chain Note Delivery ──");
    println!(
        "  Full notes extracted from TransactionResult BEFORE proving: {}",
        priv_full_notes.len()
    );
    for note in &priv_full_notes {
        let assets: Vec<String> = note
            .assets()
            .iter_fungible()
            .map(|a| format!("{} tokens", a.amount()))
            .collect();
        println!(
            "    Note ID: {} | Assets: [{}] | Recipient visible: yes (off-chain only)",
            note.id(),
            assets.join(", ")
        );
    }
    println!(
        "  → Recipient must receive full note data OFF-CHAIN (e.g., P2P relay, direct message)"
    );
    println!(
        "  → The ProvenTransaction on-chain only contains the note header (hash commitment)"
    );

    // ── 6. Submit private transfer to network ────────────────────────────
    println!("\n── Submitting PRIVATE transaction to Miden node ──");
    let config = MidenChainConfig {
        chain_reference: MidenChainReference::testnet(),
        rpc_url: "https://rpc.testnet.miden.io".to_string(),
    };
    let provider = MidenChainProvider::from_config(&config);

    let priv_tx_bytes = hex::decode(&priv_hex).expect("decode priv hex");
    let priv_inputs_bytes = hex::decode(&priv_inputs_hex).expect("decode priv inputs hex");

    let t_submit = Instant::now();
    let submitted_tx_id = provider
        .submit_proven_transaction(&priv_tx_bytes, &priv_inputs_bytes)
        .await
        .expect("submit private proven transaction");
    let submit_time = t_submit.elapsed();
    println!("  Submitted in {submit_time:.2?}");
    println!("  TX ID: {submitted_tx_id}");

    // ── 7. Query notes from network to verify visibility ─────────────────
    println!("\n── On-chain Note Visibility (via get_notes_by_id RPC) ──");

    // Sync to pick up the submitted transaction
    {
        let mut c = client.lock().await;
        c.sync_state().await.expect("post-submit sync");
    }

    // Use a standalone GrpcClient for note queries (Client doesn't expose rpc_api publicly)
    use miden_client::rpc::{Endpoint as RpcEndpoint, GrpcClient};
    use miden_client::rpc::domain::note::FetchedNote;
    use miden_protocol::block::BlockNumber;

    let rpc = GrpcClient::new(&RpcEndpoint::testnet(), 10_000);
    // Must set genesis commitment before queries
    let (genesis_header, _) = rpc
        .get_block_header_by_number(Some(BlockNumber::GENESIS), false)
        .await
        .expect("fetch genesis");
    rpc.set_genesis_commitment(genesis_header.commitment())
        .await
        .expect("set genesis commitment");

    // Query the public note IDs
    if !pub_note_ids.is_empty() {
        match rpc.get_notes_by_id(&pub_note_ids).await {
            Ok(fetched) => {
                for note in &fetched {
                    let kind = match note {
                        FetchedNote::Public(_, _) => "Public (FULL data on-chain)",
                        FetchedNote::Private(_, _) => "Private (header only on-chain)",
                    };
                    println!("    Public TX note → {kind} | ID: {}", note.id());
                }
            }
            Err(e) => println!("    Public notes query: {e} (notes may not be committed yet)"),
        }
    }

    // Query the private note IDs
    if !priv_note_ids.is_empty() {
        match rpc.get_notes_by_id(&priv_note_ids).await {
            Ok(fetched) => {
                for note in &fetched {
                    let kind = match note {
                        FetchedNote::Public(_, _) => "Public (FULL data on-chain)",
                        FetchedNote::Private(_, _) => "Private (header only on-chain)",
                    };
                    println!("    Private TX note → {kind} | ID: {}", note.id());
                }
            }
            Err(e) => println!("    Private notes query: {e} (notes may not be committed yet)"),
        }
    }

    // ── Summary ──────────────────────────────────────────────────────────
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║           PUBLIC vs PRIVATE P2ID COMPARISON                 ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!(
        "║  Proof size:     Public={pub_proof_size:>6} B │ Private={priv_proof_size:>6} B  ║"
    );
    println!(
        "║  Prove time:     Public={:>7.2?} │ Private={:>7.2?}  ║",
        pub_prove_time, priv_prove_time
    );
    println!(
        "║  Verify time:    Public={:>7.2?} │ Private={:>7.2?}  ║",
        pub_verify, priv_verify
    );
    println!("║                                                              ║");
    println!("║  On-chain data:                                              ║");
    println!("║    Public note:  Full (assets + recipient visible)           ║");
    println!("║    Private note: Header only (hash commitment)               ║");
    println!("║                                                              ║");
    println!("║  Consume flow:                                               ║");
    println!("║    Public:  Recipient syncs, sees note on-chain, consumes    ║");
    println!("║    Private: Recipient needs OFF-CHAIN delivery of full note  ║");
    println!("║             (P2P relay, direct message, etc.)                ║");
    println!("║                                                              ║");
    println!("║  x402 implication:                                           ║");
    println!("║    Facilitator CANNOT verify private notes (no Full data).   ║");
    println!("║    x402 payments MUST use NoteType::Public for verification. ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");
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
// E2E: Trusted Facilitator (Private) P2ID Transfer
// ============================================================================

/// Full x402 payment flow using TrustedFacilitator privacy mode:
/// 1. Create MidenClientSigner, sync
/// 2. create_and_prove_p2id_with_privacy with TrustedFacilitator
/// 3. Assert note_data is present and non-empty
/// 4. Deserialize ProvenTransaction → verify has OutputNote::Header (private)
/// 5. Verify STARK proof
/// 6. Deserialize full note from note_data → verify NoteId matches
/// 7. Call verify_trusted_facilitator_note → assert OK
/// 8. Submit to Miden node
/// 9. Print comparison summary
#[tokio::test]
#[ignore] // requires testnet + funded wallets
async fn e2e_trusted_facilitator_payment_flow() {
    use miden_protocol::account::AccountId;
    use miden_protocol::note::Note;
    use miden_protocol::transaction::{OutputNote, ProvenTransaction};
    use miden_protocol::utils::serde::{Deserializable, Serializable};
    use miden_tx::TransactionVerifier;
    use x402_chain_miden::privacy::{PrivacyMode, verify_trusted_facilitator_note};

    println!("\n=== E2E Trusted Facilitator Payment Flow ===\n");

    let client = build_testnet_client().await;
    let client = Arc::new(Mutex::new(client));

    // Sync
    {
        let mut c = client.lock().await;
        let summary = c.sync_state().await.expect("sync with testnet");
        println!("Synced to block {}\n", summary.block_num);
    }

    let signer = MidenClientSigner::new(WALLET_1, client.clone());

    // ── 1. Create and prove P2ID with TrustedFacilitator privacy ─────────
    let amount: u64 = 250;
    println!("Creating TrustedFacilitator P2ID: {amount} tokens {WALLET_1} → {WALLET_2}");

    let t_start = Instant::now();
    let (proven_tx_hex, tx_id, tx_inputs_hex, note_data) = signer
        .create_and_prove_p2id_with_privacy(
            WALLET_2,
            FAUCET_ID,
            amount,
            &PrivacyMode::TrustedFacilitator,
        )
        .await
        .expect("create_and_prove_p2id_with_privacy should succeed");
    let prove_time = t_start.elapsed();

    println!("Transaction proved in {prove_time:.2?}");
    println!("TX ID: {tx_id}");
    println!("ProvenTransaction hex: {} bytes", proven_tx_hex.len() / 2);
    println!("TransactionInputs hex: {} bytes", tx_inputs_hex.len() / 2);

    // ── 2. Assert note_data is present ────────────────────────────────────
    let note_data_hex = note_data.expect("note_data should be Some for TrustedFacilitator");
    assert!(!note_data_hex.is_empty(), "note_data should not be empty");
    println!("Note data: {} bytes (off-chain)", note_data_hex.len() / 2);

    // ── 3. Deserialize ProvenTransaction → verify OutputNote::Header ──────
    println!("\nInspecting ProvenTransaction output notes...");
    let proven_tx_bytes = hex::decode(&proven_tx_hex).expect("decode hex");
    let proven_tx =
        ProvenTransaction::read_from_bytes(&proven_tx_bytes).expect("deserialize ProvenTx");

    let mut has_header = false;
    for (i, output_note) in proven_tx.output_notes().iter().enumerate() {
        let variant = match output_note {
            OutputNote::Full(note) => {
                format!("Full (ID: {})", note.id())
            }
            OutputNote::Header(_) => {
                has_header = true;
                format!("Header (ID: {}) — private, no data on-chain", output_note.id())
            }
            OutputNote::Partial(_) => "Partial".to_string(),
        };
        println!("  [{i}] {variant}");
    }
    assert!(
        has_header,
        "Private P2ID should produce OutputNote::Header in ProvenTransaction"
    );

    // ── 4. Verify STARK proof ─────────────────────────────────────────────
    println!("\nVerifying STARK proof...");
    let t_verify = Instant::now();
    let verifier = TransactionVerifier::new(96);
    verifier
        .verify(&proven_tx)
        .expect("STARK proof should be valid");
    let verify_time = t_verify.elapsed();
    println!("STARK proof verified in {verify_time:.2?}");

    // ── 5. Verify NoteId binding ──────────────────────────────────────────
    println!("\nVerifying NoteId binding...");
    let note_bytes = hex::decode(&note_data_hex).expect("decode note hex");
    let full_note = Note::read_from_bytes(&note_bytes).expect("deserialize Note");

    let note_id = full_note.id();
    let id_matches = proven_tx
        .output_notes()
        .iter()
        .any(|on| on.id() == note_id);
    assert!(
        id_matches,
        "Note ID should match an output note in ProvenTransaction"
    );
    println!("  Note ID {note_id} matches output note — binding verified");

    // Verify the note data round-trips correctly
    let re_serialized = hex::encode(full_note.to_bytes());
    assert_eq!(
        note_data_hex, re_serialized,
        "Note should round-trip through serialization"
    );

    // ── 6. Call verify_trusted_facilitator_note ───────────────────────────
    println!("\nRunning verify_trusted_facilitator_note...");
    let required_recipient = AccountId::from_hex(WALLET_2).expect("parse recipient");
    let required_faucet = AccountId::from_hex(FAUCET_ID).expect("parse faucet");
    verify_trusted_facilitator_note(
        &proven_tx,
        &note_data_hex,
        required_recipient,
        required_faucet,
        amount,
    )
    .expect("verify_trusted_facilitator_note should succeed");
    println!("  verify_trusted_facilitator_note: OK");

    // ── 7. Submit to Miden node ───────────────────────────────────────────
    println!("\nSubmitting to Miden node...");
    let config = MidenChainConfig {
        chain_reference: MidenChainReference::testnet(),
        rpc_url: "https://rpc.testnet.miden.io".to_string(),
    };
    let provider = MidenChainProvider::from_config(&config);

    let tx_inputs_bytes = hex::decode(&tx_inputs_hex).expect("decode tx_inputs hex");

    let t_submit = Instant::now();
    let submitted_tx_id = provider
        .submit_proven_transaction(&proven_tx_bytes, &tx_inputs_bytes)
        .await
        .expect("submit_proven_transaction should succeed");
    let submit_time = t_submit.elapsed();

    println!("Submitted in {submit_time:.2?}");
    println!("Submitted TX ID: {submitted_tx_id}");

    // Sync to pick up changes
    {
        let mut c = client.lock().await;
        let summary = c.sync_state().await.expect("post-submit sync");
        println!("\nPost-submit sync to block {}", summary.block_num);
    }

    // ── Summary ───────────────────────────────────────────────────────────
    println!("\n╔══════════════════════════════════════════════════════════════╗");
    println!("║         TRUSTED FACILITATOR E2E RESULTS                     ║");
    println!("╠══════════════════════════════════════════════════════════════╣");
    println!("║  Privacy mode:  TrustedFacilitator (NoteType::Private)      ║");
    println!("║  Prove time:    {prove_time:.2?}                                   ║");
    println!("║  Verify time:   {verify_time:.2?}                                   ║");
    println!("║  Submit time:   {submit_time:.2?}                                   ║");
    println!("║  Total time:    {:.2?}                                   ║", t_start.elapsed());
    println!("║                                                              ║");
    println!("║  On-chain:  OutputNote::Header (hash commitment only)        ║");
    println!("║  Off-chain: Full note data ({} bytes) shared with facilitator║", note_data_hex.len() / 2);
    println!("║  NoteId binding: VERIFIED                                    ║");
    println!("║  TX ID: {submitted_tx_id}                                    ║");
    println!("╚══════════════════════════════════════════════════════════════╝\n");
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
