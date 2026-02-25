//! Tests that require the `miden-native` feature flag.
//!
//! These tests verify the integration with miden-protocol, miden-tx, and miden-standards
//! crates. They are gated behind `#[cfg(feature = "miden-native")]` so they only run
//! when the miden SDK dependencies are available.
//!
//! Note: These tests will NOT run on Windows due to the `winter-air` `aux.rs` reserved
//! filename issue. They should be run on Linux/macOS or in CI.

#![cfg(feature = "miden-native")]

use x402_chain_miden::chain::MidenAccountAddress;

// ============================================================================
// AccountId Conversion Tests
// ============================================================================

/// Test that a valid Miden account ID can be converted to a miden-protocol AccountId
/// and back without loss.
#[test]
fn test_account_id_roundtrip() {
    use miden_protocol::account::{
        AccountId, AccountIdVersion, AccountStorageMode, AccountType,
    };

    // Create a valid AccountId using the dummy constructor
    let original = AccountId::dummy(
        [42u8; 15],
        AccountIdVersion::Version0,
        AccountType::RegularAccountUpdatableCode,
        AccountStorageMode::Public,
    );

    // Convert to MidenAccountAddress
    let addr = MidenAccountAddress::from_account_id(original);

    // Convert back to AccountId
    let recovered = addr.to_account_id().expect("should parse back to AccountId");

    assert_eq!(original, recovered);
}

/// Test that from_account_id produces a valid hex string.
#[test]
fn test_from_account_id_hex_format() {
    use miden_protocol::account::{
        AccountId, AccountIdVersion, AccountStorageMode, AccountType,
    };

    let id = AccountId::dummy(
        [1u8; 15],
        AccountIdVersion::Version0,
        AccountType::FungibleFaucet,
        AccountStorageMode::Private,
    );

    let addr = MidenAccountAddress::from_account_id(id);
    let hex = addr.to_hex();

    // Should start with "0x"
    assert!(hex.starts_with("0x"), "Expected 0x prefix, got: {hex}");
    // Should be valid hex after prefix
    let without_prefix = hex.strip_prefix("0x").unwrap();
    assert!(
        hex::decode(without_prefix).is_ok(),
        "Should be valid hex: {without_prefix}"
    );
}

// ============================================================================
// ProvenTransaction Deserialization Tests
// ============================================================================

/// Test that ProvenTransaction serialization/deserialization roundtrip works.
#[test]
fn test_proven_transaction_serde_roundtrip() {
    use miden_protocol::account::{
        AccountId, AccountIdVersion, AccountStorageMode, AccountType,
    };
    use miden_protocol::asset::FungibleAsset;
    use miden_protocol::block::BlockNumber;
    use miden_protocol::transaction::ProvenTransaction;
    use miden_protocol::utils::serde::{Deserializable, Serializable};
    use miden_protocol::vm::ExecutionProof;
    use miden_protocol::Word;

    // Build a minimal ProvenTransaction for testing deserialization.
    // This uses the builder pattern from the protocol crate.
    let account_id = AccountId::dummy(
        [1; 15],
        AccountIdVersion::Version0,
        AccountType::FungibleFaucet,
        AccountStorageMode::Private,
    );

    let initial = [2u8; 32]
        .try_into()
        .expect("valid initial commitment");
    let final_commit = [3u8; 32]
        .try_into()
        .expect("valid final commitment");
    let delta_commit = [4u8; 32]
        .try_into()
        .expect("valid delta commitment");
    let ref_block_num = BlockNumber::from(1);
    let ref_block_commitment = Word::default();
    let expiration = BlockNumber::from(100);
    let proof = ExecutionProof::new_dummy();
    let fee = FungibleAsset::mock(42).unwrap_fungible();

    let tx = miden_protocol::transaction::ProvenTransactionBuilder::new(
        account_id,
        initial,
        final_commit,
        delta_commit,
        ref_block_num,
        ref_block_commitment,
        fee,
        expiration,
        proof,
    )
    .build()
    .expect("should build ProvenTransaction");

    // Serialize to bytes
    let bytes = tx.to_bytes();
    assert!(!bytes.is_empty(), "Serialized bytes should not be empty");

    // Deserialize back
    let recovered =
        ProvenTransaction::read_from_bytes(&bytes).expect("should deserialize ProvenTransaction");

    assert_eq!(tx.id(), recovered.id());
    assert_eq!(tx.account_id(), recovered.account_id());
}

/// Test that hex encode/decode roundtrip works for ProvenTransaction bytes.
#[test]
fn test_proven_transaction_hex_roundtrip() {
    use miden_protocol::account::{
        AccountId, AccountIdVersion, AccountStorageMode, AccountType,
    };
    use miden_protocol::asset::FungibleAsset;
    use miden_protocol::block::BlockNumber;
    use miden_protocol::transaction::ProvenTransaction;
    use miden_protocol::utils::serde::{Deserializable, Serializable};
    use miden_protocol::vm::ExecutionProof;
    use miden_protocol::Word;

    let account_id = AccountId::dummy(
        [7; 15],
        AccountIdVersion::Version0,
        AccountType::RegularAccountUpdatableCode,
        AccountStorageMode::Public,
    );

    let tx = miden_protocol::transaction::ProvenTransactionBuilder::new(
        account_id,
        [5u8; 32].try_into().unwrap(),
        [6u8; 32].try_into().unwrap(),
        [7u8; 32].try_into().unwrap(),
        BlockNumber::from(10),
        Word::default(),
        FungibleAsset::mock(100).unwrap_fungible(),
        BlockNumber::from(200),
        ExecutionProof::new_dummy(),
    )
    .build()
    .expect("should build");

    // Simulate the wire format: serialize → hex encode → hex decode → deserialize
    let bytes = tx.to_bytes();
    let hex_str = hex::encode(&bytes);
    let decoded_bytes = hex::decode(&hex_str).expect("should decode hex");
    let recovered =
        ProvenTransaction::read_from_bytes(&decoded_bytes).expect("should deserialize");

    assert_eq!(tx.id(), recovered.id());
}

// ============================================================================
// TransactionVerifier Tests
// ============================================================================

/// Test that TransactionVerifier can be instantiated with the standard security level.
#[test]
fn test_transaction_verifier_creation() {
    let _verifier = miden_tx::TransactionVerifier::new(96);
}

// ============================================================================
// P2ID Note Tests
// ============================================================================

/// Test that P2idNote script root is consistent across calls.
#[test]
fn test_p2id_script_root_consistency() {
    let root1 = miden_standards::note::P2idNote::script_root();
    let root2 = miden_standards::note::P2idNote::script_root();
    assert_eq!(root1, root2, "P2ID script root should be deterministic");
}

/// Test P2idNoteStorage roundtrip via AccountId.
#[test]
fn test_p2id_note_storage_roundtrip() {
    use miden_protocol::account::{
        AccountId, AccountIdVersion, AccountStorageMode, AccountType,
    };
    use miden_protocol::note::NoteStorage;
    use miden_standards::note::P2idNoteStorage;

    let target = AccountId::dummy(
        [99u8; 15],
        AccountIdVersion::Version0,
        AccountType::RegularAccountUpdatableCode,
        AccountStorageMode::Public,
    );

    let storage = P2idNoteStorage::new(target);
    let note_storage: NoteStorage = storage.into();
    let recovered =
        P2idNoteStorage::try_from(note_storage.items()).expect("should parse P2ID storage");

    assert_eq!(recovered.target(), target);
}
