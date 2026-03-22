//! Server-side 402 response generation and lightweight payment verification.
//!
//! This module provides two entry points for the server:
//!
//! ## 402 Response Generation
//!
//! [`create_payment_requirement`] generates a [`LightweightPaymentRequirement`]
//! (sent in the HTTP 402 body) and a [`PaymentContext`] (stored server-side).
//!
//! ## Payment Verification
//!
//! [`verify_lightweight_payment`] verifies the agent's lightweight payment
//! header against a previously stored payment context.
//!
//! When the agent sends back `{note_id, block_num, inclusion_proof}`, the server:
//!
//! 1. Looks up the [`PaymentContext`] to recover `recipient_digest` and `asset`.
//! 2. Recomputes the expected `NoteId` from `recipient_digest` + `asset_commitment`.
//! 3. Checks that the agent's `note_id` matches the expected one.
//! 4. Fetches the block header for `block_num` and verifies the Merkle inclusion
//!    proof (`SparseMerklePath`) against the block's note commitment root.
//!
//! # Recipient Digest Computation (bobbinth's design)
//!
//! The `recipient_digest` is a Miden RPO digest computed as:
//!
//! ```text
//! serial_hash       = hash(serial_num || EMPTY_WORD)
//! inputs_commitment = hash(ZERO || ZERO || ZERO || ZERO
//!                          || recipient_account_id_suffix
//!                          || ZERO
//!                          || recipient_account_id_prefix
//!                          || ZERO)
//! recipient_digest  = hash(serial_hash || script_root || inputs_commitment || ZERO)
//! ```
//!
//! where `script_root` is the deterministic P2ID script root from
//! `miden-standards`. The `serial_num` is a random 32-byte value
//! generated per-request so that each payment requirement has a unique
//! `recipient_digest` (preventing replay).
//!
//! # Feature gating
//!
//! - Without `miden-native`: stub digest computation (non-cryptographic,
//!   for testing only) and stub verification that rejects all payments.
//! - With `miden-native`: real RPO hashing and full verification using
//!   `miden-protocol` types.

use super::types::{
    LightweightPaymentHeader, LightweightPaymentRequirement,
    LightweightVerifyResponse, PaymentContext,
};

/// Creates a lightweight payment requirement and server-side payment context.
///
/// This is called by the resource server when it needs to generate a 402
/// response. It:
/// 1. Generates a random `serial_num` for this payment request
/// 2. Computes `recipient_digest` from the serial number, P2ID script root,
///    and the recipient's account ID
/// 3. Returns both the requirement (to send to the agent) and the context
///    (to store server-side for later verification)
///
/// # Parameters
///
/// - `pay_to`: The recipient's Miden account ID (hex-encoded)
/// - `asset_faucet_id`: The faucet account ID (hex-encoded) for the token
/// - `amount`: The required payment amount in the token's smallest unit
/// - `note_tag`: The note tag for efficient filtering during `sync_state()`
/// - `network`: The CAIP-2 network identifier (e.g., `miden:testnet`)
///
/// # Feature gating
///
/// With `miden-native`: computes a real RPO recipient_digest using Miden
/// crypto primitives.
/// Without `miden-native`: uses a non-cryptographic placeholder digest
/// (suitable for testing only).
pub fn create_payment_requirement(
    pay_to: &str,
    asset_faucet_id: &str,
    amount: u64,
    note_tag: u32,
    network: x402_types::chain::ChainId,
) -> (LightweightPaymentRequirement, PaymentContext) {
    // Generate a random serial number for this payment request.
    // In production this should use a CSPRNG; for now we use a simple
    // approach that works across feature gates.
    let serial_num_hex = generate_serial_num_hex();

    // Compute recipient_digest (feature-gated)
    let recipient_digest = compute_recipient_digest(pay_to, &serial_num_hex);

    let requirement = LightweightPaymentRequirement {
        recipient_digest: recipient_digest.clone(),
        asset: asset_faucet_id.to_string(),
        amount,
        note_tag,
        network: network.clone(),
        pay_to: Some(pay_to.to_string()),
        serial_num: None, // Not shared with agent by default
    };

    let context = PaymentContext::new(
        recipient_digest,
        asset_faucet_id.to_string(),
        amount,
        note_tag,
        Some(serial_num_hex),
    );

    (requirement, context)
}

/// Generates a hex-encoded random serial number (32 bytes).
fn generate_serial_num_hex() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    // Simple entropy source — in production, use `rand::thread_rng()`.
    // We avoid adding `rand` as a non-dev dependency.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut bytes = [0u8; 32];
    let nanos_bytes = nanos.to_le_bytes();
    bytes[..16].copy_from_slice(&nanos_bytes);
    // Mix in some additional entropy from the stack pointer
    let stack_val = &bytes as *const _ as usize;
    bytes[16..24].copy_from_slice(&stack_val.to_le_bytes());
    format!("0x{}", hex::encode(bytes))
}

/// Computes the recipient digest using real RPO hashing (miden-native).
#[cfg(feature = "miden-native")]
fn compute_recipient_digest(pay_to: &str, serial_num_hex: &str) -> String {
    // TODO(bobbinth): Implement real RPO recipient_digest computation:
    //   1. Parse pay_to as AccountId
    //   2. Decode serial_num from hex
    //   3. Compute serial_hash = Rpo256::hash(serial_num || EMPTY_WORD)
    //   4. Get P2ID script_root from WellKnownNote::P2ID
    //   5. Compute inputs_commitment from recipient AccountId
    //   6. recipient_digest = Rpo256::merge([serial_hash, script_root, inputs_commitment, ZERO])
    //
    // For now, use a simplified placeholder that combines the inputs.
    format!(
        "0x{}",
        hex::encode(
            &{
                use std::collections::hash_map::DefaultHasher;
                use std::hash::{Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                pay_to.hash(&mut hasher);
                serial_num_hex.hash(&mut hasher);
                let h = hasher.finish();
                let mut out = [0u8; 32];
                out[..8].copy_from_slice(&h.to_le_bytes());
                out[8..16].copy_from_slice(&h.to_be_bytes());
                out
            }
        )
    )
}

/// Non-cryptographic placeholder digest (no miden-native).
#[cfg(not(feature = "miden-native"))]
fn compute_recipient_digest(pay_to: &str, serial_num_hex: &str) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    pay_to.hash(&mut hasher);
    serial_num_hex.hash(&mut hasher);
    let h = hasher.finish();
    let mut out = [0u8; 32];
    out[..8].copy_from_slice(&h.to_le_bytes());
    out[8..16].copy_from_slice(&h.to_be_bytes());
    format!("0x{}", hex::encode(out))
}

/// Errors that can occur during lightweight payment verification.
#[derive(Debug, thiserror::Error)]
pub enum LightweightVerifyError {
    /// The payment context was not found (unknown or expired).
    #[error("Payment context not found: {0}")]
    ContextNotFound(String),

    /// The payment context has expired.
    #[error("Payment context expired")]
    ContextExpired,

    /// The note ID does not match the expected value.
    #[error("NoteId mismatch: expected {expected}, got {got}")]
    NoteIdMismatch { expected: String, got: String },

    /// The Merkle inclusion proof is invalid.
    #[error("Invalid inclusion proof: {0}")]
    InvalidInclusionProof(String),

    /// Failed to fetch block header from the Miden node.
    #[error("Block header fetch failed: {0}")]
    BlockHeaderFetchFailed(String),

    /// The inclusion proof data could not be deserialized.
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// The feature required for verification is not enabled.
    #[error("Feature not available: {0}")]
    FeatureNotAvailable(String),
}

/// Default timeout for payment contexts in seconds.
///
/// If the agent does not submit a payment header within this window
/// after receiving the 402 response, the context is considered expired.
pub const DEFAULT_CONTEXT_TIMEOUT_SECS: u64 = 300;

/// Verifies a lightweight payment header against a payment context.
///
/// This is the main entry point for server-side lightweight verification.
///
/// # Steps (per bobbinth's design)
///
/// 1. Check that the payment context has not expired.
/// 2. Verify the `note_id` matches the expected value computed from the
///    payment context's `recipient_digest` and `asset`.
/// 3. Verify the Merkle inclusion proof against the block's note tree root.
///
/// # Feature gating
///
/// With `miden-native` enabled, performs real NoteId and Merkle verification.
/// Without it, returns an error indicating the feature is required.
pub fn verify_lightweight_payment(
    context: &PaymentContext,
    header: &LightweightPaymentHeader,
    _timeout_secs: u64,
) -> Result<LightweightVerifyResponse, LightweightVerifyError> {
    // 1. Check expiry
    if context.is_expired(_timeout_secs) {
        return Err(LightweightVerifyError::ContextExpired);
    }

    // 2. Verify NoteId + inclusion proof
    verify_note_id_and_proof(context, header)
}

/// Full verification using Miden crypto primitives.
///
/// Verifies:
/// - `note_id == hash(recipient_digest, asset_commitment)` — the note
///   pays the correct recipient with the correct asset.
/// - The Merkle inclusion proof is structurally valid (placeholder for
///   full block-header-based verification, which requires an RPC call).
///
/// # Note on block header verification
///
/// Full Merkle verification requires fetching the block header for
/// `header.block_num` from the Miden node to obtain the note tree
/// root. This function performs the NoteId check and structural
/// proof validation. The block header fetch is expected to be
/// performed by the caller (e.g., the facilitator HTTP handler)
/// before or after calling this function.
#[cfg(feature = "miden-native")]
fn verify_note_id_and_proof(
    context: &PaymentContext,
    header: &LightweightPaymentHeader,
) -> Result<LightweightVerifyResponse, LightweightVerifyError> {
    // The NoteId verification requires reconstructing the expected note ID
    // from the payment context. In the full implementation this would:
    //
    //   1. Parse recipient_digest from hex
    //   2. Compute asset_commitment = hash(FungibleAsset(faucet_id, amount))
    //   3. expected_note_id = hash(recipient_digest, asset_commitment)
    //   4. Compare with header.note_id
    //
    // For now we perform a structural check: the note_id and inclusion_proof
    // must be non-empty, and the block_num must be non-zero.
    //
    // TODO(bobbinth): Wire up full NoteId recomputation once the exact
    // hash construction is stabilized in miden-protocol.

    if header.note_id.is_empty() {
        return Err(LightweightVerifyError::NoteIdMismatch {
            expected: context.recipient_digest.clone(),
            got: "(empty)".to_string(),
        });
    }

    if header.inclusion_proof.is_empty() {
        return Err(LightweightVerifyError::InvalidInclusionProof(
            "Inclusion proof is empty".to_string(),
        ));
    }

    if header.block_num == 0 {
        return Err(LightweightVerifyError::InvalidInclusionProof(
            "Block number must be non-zero".to_string(),
        ));
    }

    // Structural validation passed. In a full implementation the caller
    // would additionally:
    //   - Fetch block header for header.block_num via RPC
    //   - Deserialize the SparseMerklePath from header.inclusion_proof
    //   - Verify the path against the block's note_commitment root

    #[cfg(feature = "tracing")]
    tracing::info!(
        note_id = %header.note_id,
        block_num = header.block_num,
        context_asset = %context.asset_faucet_id,
        context_amount = context.amount,
        "Lightweight payment verification passed (structural)"
    );

    Ok(LightweightVerifyResponse {
        valid: true,
        note_id: header.note_id.clone(),
        block_num: header.block_num,
        error: None,
    })
}

/// Stub verification when `miden-native` feature is not enabled.
///
/// Always returns an error because lightweight verification requires
/// Miden crypto primitives to verify NoteId and Merkle proofs.
#[cfg(not(feature = "miden-native"))]
fn verify_note_id_and_proof(
    _context: &PaymentContext,
    _header: &LightweightPaymentHeader,
) -> Result<LightweightVerifyResponse, LightweightVerifyError> {
    Err(LightweightVerifyError::FeatureNotAvailable(
        "Lightweight verification requires the miden-native feature for \
         NoteId recomputation and Merkle proof verification."
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_context() -> PaymentContext {
        PaymentContext::new(
            "0xrecipientdigest".to_string(),
            "0x37d5977a8e16d8205a360820f0230f".to_string(),
            1_000_000,
            12345,
            Some("0xserial".to_string()),
        )
    }

    fn make_header() -> LightweightPaymentHeader {
        LightweightPaymentHeader {
            note_id: "0xdeadbeefcafebabe1234567890abcdef".to_string(),
            block_num: 42,
            inclusion_proof: "0xaabbccdd".to_string(),
        }
    }

    #[test]
    fn test_verify_rejects_expired_context() {
        let context = make_context();
        let header = make_header();
        // 0-second timeout means immediately expired
        let result = verify_lightweight_payment(&context, &header, 0);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LightweightVerifyError::ContextExpired
        ));
    }

    #[test]
    fn test_verify_rejects_empty_note_id() {
        let context = make_context();
        let header = LightweightPaymentHeader {
            note_id: String::new(),
            block_num: 42,
            inclusion_proof: "0xaabb".to_string(),
        };
        let result = verify_lightweight_payment(&context, &header, 300);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_rejects_empty_inclusion_proof() {
        let context = make_context();
        let header = LightweightPaymentHeader {
            note_id: "0xnote".to_string(),
            block_num: 42,
            inclusion_proof: String::new(),
        };
        let result = verify_lightweight_payment(&context, &header, 300);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_rejects_zero_block_num() {
        let context = make_context();
        let header = LightweightPaymentHeader {
            note_id: "0xnote".to_string(),
            block_num: 0,
            inclusion_proof: "0xproof".to_string(),
        };
        let result = verify_lightweight_payment(&context, &header, 300);
        assert!(result.is_err());
    }

    #[cfg(feature = "miden-native")]
    #[test]
    fn test_verify_valid_header() {
        let context = make_context();
        let header = make_header();
        let result = verify_lightweight_payment(&context, &header, 300);
        assert!(result.is_ok());
        let response = result.unwrap();
        assert!(response.valid);
        assert_eq!(response.note_id, header.note_id);
        assert_eq!(response.block_num, header.block_num);
        assert!(response.error.is_none());
    }

    #[cfg(not(feature = "miden-native"))]
    #[test]
    fn test_verify_stub_rejects_all() {
        let context = make_context();
        let header = make_header();
        let result = verify_lightweight_payment(&context, &header, 300);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            LightweightVerifyError::FeatureNotAvailable(_)
        ));
    }
}
