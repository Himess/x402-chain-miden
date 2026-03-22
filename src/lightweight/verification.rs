//! Lightweight payment verification using note inclusion proofs.
//!
//! Instead of verifying a full STARK proof (which requires the agent to send
//! the entire `ProvenTransaction` ~100 KB), this module verifies payment by
//! checking:
//!
//! 1. The `NoteId` matches the expected value (computed from `recipient_digest` +
//!    `asset_commitment`)
//! 2. The note is included in the block's note tree (`SparseMerklePath`)
//! 3. The block is part of the canonical chain (via cached block headers / MMR)
//!
//! This implements the design proposed by bobbinth in 0xMiden/node#1796.
//!
//! # Verification Steps
//!
//! ```text
//! PaymentContext (server-side)     LightweightPaymentHeader (agent-side)
//! ┌──────────────────────┐        ┌──────────────────────┐
//! │ recipient_digest     │        │ note_id              │
//! │ asset_faucet_id      │        │ block_num            │
//! │ amount               │        │ note_index           │
//! └──────────────────────┘        │ note_metadata        │
//!                                 │ inclusion_proof      │
//!                                 └──────────────────────┘
//!           │                                │
//!           ▼                                ▼
//!  ┌─────────────────────────────────────────────────┐
//!  │ 1. Check expiry                                  │
//!  │ 2. expected_note_id = hash(recipient, asset)     │
//!  │ 3. assert note_id == expected_note_id            │
//!  │ 4. Fetch block header (cache or RPC)             │
//!  │ 5. SparseMerklePath.verify(note_root)            │
//!  │ 6. Return LightweightVerifyResponse              │
//!  └─────────────────────────────────────────────────┘
//! ```

use super::chain_state::FacilitatorChainState;
use super::types::{LightweightPaymentHeader, LightweightVerifyResponse, PaymentContext};
use crate::v2_miden_exact::types::MidenExactError;

/// Default timeout (in seconds) for payment contexts when none is specified.
#[cfg(feature = "miden-native")]
const DEFAULT_PAYMENT_TIMEOUT_SECS: u64 = 300;

/// Verifies a lightweight payment header against a payment context.
///
/// This implements bobbinth's design from 0xMiden/node#1796:
///
/// 1. Check that the payment context has not expired.
/// 2. Reconstruct `expected_note_id = hash(recipient_digest, asset_commitment)`:
///    - Parse `recipient_digest` from hex into a `Word`
///    - Compute the asset commitment from `FungibleAsset::new(faucet_id, amount)`
///    - Compute the `NoteId` using miden-protocol's hashing
/// 3. Compare the agent's `note_id` with the expected value.
/// 4. Get the block header for `block_num` from the chain state cache (falls
///    back to RPC if not cached).
/// 5. Verify the `SparseMerklePath` (the `inclusion_proof`) against the block's
///    `note_root`.
/// 6. Return a [`LightweightVerifyResponse`].
///
/// # Arguments
///
/// * `payment_context` - Server-side context with `recipient_digest` and asset info,
///   created when the 402 response was issued.
/// * `payment_header` - Agent-submitted `{note_id, block_num, inclusion_proof}`.
/// * `chain_state` - Cached block headers for `note_root` lookup and MMR checks.
///
/// # Feature Gates
///
/// The `miden-native` feature is required for the cryptographic operations
/// (NoteId reconstruction, SparseMerklePath verification). Without it, this
/// function returns an error.
#[cfg(feature = "miden-native")]
pub async fn verify_lightweight_payment(
    payment_context: &PaymentContext,
    payment_header: &LightweightPaymentHeader,
    chain_state: &FacilitatorChainState,
) -> Result<LightweightVerifyResponse, MidenExactError> {
    use miden_protocol::Word;
    use miden_protocol::account::AccountId;
    use miden_protocol::asset::FungibleAsset;
    use miden_protocol::crypto::merkle::SparseMerklePath;
    use miden_protocol::note::{NoteId, NoteMetadata, compute_note_commitment};
    use miden_protocol::utils::serde::Deserializable;

    // ------------------------------------------------------------------
    // 1. Check that the payment context has not expired.
    // ------------------------------------------------------------------
    if payment_context.is_expired(DEFAULT_PAYMENT_TIMEOUT_SECS) {
        return Err(MidenExactError::TransactionExpired(
            DEFAULT_PAYMENT_TIMEOUT_SECS,
        ));
    }

    // ------------------------------------------------------------------
    // 2. Reconstruct the expected NoteId.
    //
    //    NoteId = hash(recipient_digest, asset_commitment)
    //
    //    The recipient_digest was computed server-side when the 402 response
    //    was generated. The asset_commitment is derived from the faucet ID
    //    and amount in the payment context.
    // ------------------------------------------------------------------

    // 2a. Parse recipient_digest from hex -> Word
    let recipient_digest_hex = payment_context
        .recipient_digest
        .strip_prefix("0x")
        .unwrap_or(&payment_context.recipient_digest);

    let recipient_digest_bytes = hex::decode(recipient_digest_hex).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in recipient_digest: {e}"))
    })?;

    let recipient_digest = Word::read_from_bytes(&recipient_digest_bytes).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Failed to deserialize recipient_digest as Word: {e}"
        ))
    })?;

    // 2b. Parse faucet account ID
    let faucet_id = AccountId::from_hex(&payment_context.asset_faucet_id).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Invalid faucet account ID '{}': {e}",
            payment_context.asset_faucet_id
        ))
    })?;

    // 2c. Compute asset commitment from FungibleAsset
    let asset = FungibleAsset::new(faucet_id, payment_context.amount).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Failed to create FungibleAsset(faucet={}, amount={}): {e}",
            payment_context.asset_faucet_id, payment_context.amount
        ))
    })?;

    // 2d. Reconstruct the expected NoteId
    //     NoteId internally hashes the recipient_digest with the asset commitment.
    let expected_note_id = reconstruct_note_id(&recipient_digest, &asset)?;

    // ------------------------------------------------------------------
    // 3. Compare the agent's note_id with the expected value.
    // ------------------------------------------------------------------
    let expected_hex = format!("{expected_note_id}");
    let agent_note_id_normalized = normalize_hex_string(&payment_header.note_id);
    let expected_note_id_normalized = normalize_hex_string(&expected_hex);

    if agent_note_id_normalized != expected_note_id_normalized {
        return Err(MidenExactError::NoteIdMismatch {
            expected: expected_hex,
            got: payment_header.note_id.clone(),
        });
    }

    // ------------------------------------------------------------------
    // 4. Get the block header from the chain state cache.
    //
    //    The chain state caches block headers by block number. If the
    //    block is not cached, it falls back to an RPC call.
    // ------------------------------------------------------------------
    let cached_header = chain_state
        .get_block_header(payment_header.block_num)
        .await?;

    // ------------------------------------------------------------------
    // 5. Verify the SparseMerklePath against the block's note_root.
    //
    //    The inclusion_proof is a hex-encoded SparseMerklePath that the
    //    agent obtained via sync_state() after transaction inclusion.
    // ------------------------------------------------------------------
    let proof_hex = payment_header
        .inclusion_proof
        .strip_prefix("0x")
        .unwrap_or(&payment_header.inclusion_proof);

    let proof_bytes = hex::decode(proof_hex).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in inclusion_proof: {e}"))
    })?;

    let merkle_path = SparseMerklePath::read_from_bytes(&proof_bytes).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Failed to deserialize SparseMerklePath: {e}"
        ))
    })?;

    // Parse the block's note_root as a Word for Merkle verification.
    // The note_root is stored as a hex string produced by Word::to_hex()
    // (which includes the "0x" prefix).
    let expected_root = Word::try_from(cached_header.note_root.as_str()).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Failed to parse cached note_root as Word: {e}"
        ))
    })?;

    // Parse the agent's NoteId from hex so we can compute the note commitment.
    let agent_note_id = NoteId::try_from_hex(&payment_header.note_id).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Failed to parse agent note_id '{}' as NoteId: {e}",
            payment_header.note_id
        ))
    })?;

    // Parse the note metadata from the agent's hex-encoded serialized NoteMetadata.
    // The note metadata is needed to compute the note commitment:
    //   note_commitment = hash(note_id || metadata_commitment)
    let metadata_hex = payment_header
        .note_metadata
        .strip_prefix("0x")
        .unwrap_or(&payment_header.note_metadata);

    let metadata_bytes = hex::decode(metadata_hex).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in note_metadata: {e}"))
    })?;

    let note_metadata = NoteMetadata::read_from_bytes(&metadata_bytes).map_err(|e| {
        MidenExactError::DeserializationError(format!("Failed to deserialize NoteMetadata: {e}"))
    })?;

    // Compute the note commitment, which is the leaf value stored in the
    // block's note tree at the position `note_index`.
    //   note_commitment = hash(note_id_word || metadata_commitment)
    let note_commitment = compute_note_commitment(agent_note_id, &note_metadata);

    // Verify the SparseMerklePath: check that hashing up from the leaf
    // (note_commitment at index note_index) produces the expected note_root.
    merkle_path
        .verify(
            payment_header.note_index as u64,
            note_commitment,
            &expected_root,
        )
        .map_err(|e| {
            MidenExactError::InclusionProofInvalid(format!(
                "SparseMerklePath verification failed for note_index={}: {e}",
                payment_header.note_index
            ))
        })?;

    #[cfg(feature = "tracing")]
    tracing::info!(
        note_id = %payment_header.note_id,
        block_num = %payment_header.block_num,
        note_index = %payment_header.note_index,
        "Lightweight payment verification passed: NoteId matches, Merkle inclusion verified"
    );

    // ------------------------------------------------------------------
    // 6. Return success response.
    // ------------------------------------------------------------------
    Ok(LightweightVerifyResponse {
        valid: true,
        note_id: payment_header.note_id.clone(),
        block_num: payment_header.block_num,
        error: None,
    })
}

/// Non-native stub — rejects all payments because cryptographic verification
/// is unavailable without the `miden-native` feature.
#[cfg(not(feature = "miden-native"))]
pub async fn verify_lightweight_payment(
    _payment_context: &PaymentContext,
    _payment_header: &LightweightPaymentHeader,
    _chain_state: &FacilitatorChainState,
) -> Result<LightweightVerifyResponse, MidenExactError> {
    Err(MidenExactError::InvalidProof(
        "Lightweight verification requires the miden-native feature. \
         Enable it in Cargo.toml: x402-chain-miden = { features = [\"miden-native\"] }"
            .to_string(),
    ))
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Reconstructs a `NoteId` from a recipient digest and a fungible asset.
///
/// This mirrors the Miden protocol's NoteId computation:
/// `NoteId = hash(recipient_digest, asset_commitment)`
///
/// The `FungibleAsset` is converted to a `Word` (4 field elements) which
/// serves as the asset commitment in the NoteId hash.
#[cfg(feature = "miden-native")]
fn reconstruct_note_id(
    recipient_digest: &miden_protocol::Word,
    asset: &miden_protocol::asset::FungibleAsset,
) -> Result<miden_protocol::note::NoteId, MidenExactError> {
    use miden_protocol::asset::Asset;
    use miden_protocol::note::{NoteAssets, NoteId};

    // The NoteId in Miden is computed from the full note header:
    //   NoteId = hash(recipient, asset_commitment, metadata)
    //
    // Since we're reconstructing without full metadata, we compute
    // the NoteId by building a minimal note structure with just the
    // recipient and asset. The actual NoteId computation uses the
    // NoteId::new() constructor which takes the recipient digest and
    // asset commitment word.
    //
    // NOTE: The exact NoteId reconstruction depends on the miden-protocol
    // version. In 0.13, NoteId is derived from the full note hash.
    // The approach here constructs a NoteAssets from the single fungible
    // asset, computes its commitment, then combines with the recipient
    // digest to produce the expected NoteId.
    let note_assets = NoteAssets::new(vec![Asset::Fungible(*asset)]).map_err(|e| {
        MidenExactError::DeserializationError(format!(
            "Failed to create NoteAssets from fungible asset: {e}"
        ))
    })?;

    let asset_commitment = note_assets.commitment();

    // NoteId = hash(recipient_digest, asset_commitment)
    // This is what the Miden protocol uses internally.
    let note_id = NoteId::new(*recipient_digest, asset_commitment);

    Ok(note_id)
}

/// Normalizes a hex string by stripping the `0x` prefix and lowercasing.
///
/// Used for case-insensitive NoteId comparison between the agent's
/// submitted value and the server's reconstructed expected value.
#[cfg(any(feature = "miden-native", test))]
fn normalize_hex_string(s: &str) -> String {
    s.strip_prefix("0x").unwrap_or(s).to_lowercase()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_hex_string() {
        assert_eq!(normalize_hex_string("0xABCDEF"), "abcdef");
        assert_eq!(normalize_hex_string("ABCDEF"), "abcdef");
        assert_eq!(normalize_hex_string("0xabcdef"), "abcdef");
        assert_eq!(normalize_hex_string("abcdef"), "abcdef");
    }

    #[test]
    fn test_normalize_hex_preserves_content() {
        let a = normalize_hex_string("0xDeAdBeEf");
        let b = normalize_hex_string("deadbeef");
        assert_eq!(a, b);
    }

    #[cfg(not(feature = "miden-native"))]
    #[tokio::test]
    async fn test_verify_stub_rejects_without_native() {
        use crate::chain::MidenChainReference;

        let ctx = PaymentContext::new(
            "0xaabb".to_string(),
            "0xccdd".to_string(),
            1_000_000,
            42,
            None,
        );
        let header = LightweightPaymentHeader {
            note_id: "0xdeadbeef".to_string(),
            block_num: 10,
            note_index: 0,
            note_metadata: "0xaabb".to_string(),
            inclusion_proof: "0xcafe".to_string(),
        };
        let chain_state = FacilitatorChainState::new(
            "https://rpc.testnet.miden.io".to_string(),
            MidenChainReference::testnet(),
        );

        let result = verify_lightweight_payment(&ctx, &header, &chain_state).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, MidenExactError::InvalidProof(_)));
    }

    #[test]
    fn test_payment_context_expiry_check() {
        let ctx = PaymentContext::new(
            "0xaabb".to_string(),
            "0xccdd".to_string(),
            1_000_000,
            42,
            None,
        );
        // Just created — should not be expired with a 300-second timeout
        assert!(!ctx.is_expired(300));
    }
}
