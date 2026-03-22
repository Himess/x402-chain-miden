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

use miden_client::builder::ClientBuilder;
use miden_client::keystore::FilesystemKeyStore;
use miden_client::rpc::Endpoint;
use miden_client::Client;
use miden_client_sqlite_store::SqliteStore;
use tokio::sync::Mutex;

use x402_chain_miden::chain::{MidenAccountAddress, MidenChainConfig, MidenChainProvider, MidenChainReference};

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
