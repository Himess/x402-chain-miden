//! Tests that require the `miden-native` feature flag.
//!
//! These tests verify the integration with miden-protocol, miden-tx, and miden-standards
//! crates. They are gated behind `#[cfg(feature = "miden-native")]` so they only run
//! when the miden SDK dependencies are available.

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

    let original = AccountId::dummy(
        [42u8; 15],
        AccountIdVersion::Version0,
        AccountType::RegularAccountUpdatableCode,
        AccountStorageMode::Public,
    );

    let addr = MidenAccountAddress::from_account_id(original);
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

    assert!(hex.starts_with("0x"), "Expected 0x prefix, got: {hex}");
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

    let account_id = AccountId::dummy(
        [1; 15],
        AccountIdVersion::Version0,
        AccountType::FungibleFaucet,
        AccountStorageMode::Private,
    );

    let faucet_id = AccountId::dummy(
        [2; 15],
        AccountIdVersion::Version0,
        AccountType::FungibleFaucet,
        AccountStorageMode::Public,
    );

    let tx = miden_protocol::transaction::ProvenTransactionBuilder::new(
        account_id,
        [2u8; 32].try_into().unwrap(),
        [3u8; 32].try_into().unwrap(),
        [4u8; 32].try_into().unwrap(),
        BlockNumber::from(1),
        Word::default(),
        FungibleAsset::new(faucet_id, 42).expect("valid asset"),
        BlockNumber::from(100),
        ExecutionProof::new_dummy(),
    )
    .build()
    .expect("should build ProvenTransaction");

    let bytes = tx.to_bytes();
    assert!(!bytes.is_empty(), "Serialized bytes should not be empty");

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
        AccountStorageMode::Private,
    );

    let faucet_id = AccountId::dummy(
        [8; 15],
        AccountIdVersion::Version0,
        AccountType::FungibleFaucet,
        AccountStorageMode::Public,
    );

    let tx = miden_protocol::transaction::ProvenTransactionBuilder::new(
        account_id,
        [5u8; 32].try_into().unwrap(),
        [6u8; 32].try_into().unwrap(),
        [7u8; 32].try_into().unwrap(),
        BlockNumber::from(10),
        Word::default(),
        FungibleAsset::new(faucet_id, 100).expect("valid asset"),
        BlockNumber::from(200),
        ExecutionProof::new_dummy(),
    )
    .build()
    .expect("should build");

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

/// Test that WellKnownNote::P2ID script root is consistent across calls.
#[test]
fn test_p2id_script_root_consistency() {
    use miden_standards::note::WellKnownNote;

    let root1 = WellKnownNote::P2ID.script_root();
    let root2 = WellKnownNote::P2ID.script_root();
    assert_eq!(root1, root2, "P2ID script root should be deterministic");
}

/// Test that P2ID note can be created and its inputs contain the target account.
#[test]
fn test_p2id_note_target_extraction() {
    use miden_protocol::account::{
        AccountId, AccountIdVersion, AccountStorageMode, AccountType,
    };
    use miden_protocol::asset::{Asset, FungibleAsset};
    use miden_protocol::note::NoteType;
    use miden_standards::note::create_p2id_note;

    let sender = AccountId::dummy(
        [10u8; 15],
        AccountIdVersion::Version0,
        AccountType::RegularAccountUpdatableCode,
        AccountStorageMode::Public,
    );

    let target = AccountId::dummy(
        [99u8; 15],
        AccountIdVersion::Version0,
        AccountType::RegularAccountUpdatableCode,
        AccountStorageMode::Public,
    );

    let faucet = AccountId::dummy(
        [50u8; 15],
        AccountIdVersion::Version0,
        AccountType::FungibleFaucet,
        AccountStorageMode::Public,
    );

    let asset = FungibleAsset::new(faucet, 1_000_000).expect("valid asset");
    let mut rng = miden_protocol::crypto::rand::RpoRandomCoin::new(miden_protocol::Word::default());

    let note = create_p2id_note(
        sender,
        target,
        vec![Asset::Fungible(asset)],
        NoteType::Public,
        Default::default(),
        &mut rng,
    )
    .expect("should create P2ID note");

    // Verify the note inputs contain the target account ID
    // build_p2id_recipient stores [target.suffix(), target.prefix().as_felt()]
    let inputs = note.recipient().inputs().values();
    assert!(inputs.len() >= 2, "P2ID note should have at least 2 inputs");

    let recovered_target = AccountId::new_unchecked([inputs[1], inputs[0]]);
    assert_eq!(recovered_target, target, "Extracted target should match original");
}
