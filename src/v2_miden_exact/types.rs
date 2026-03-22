//! Type definitions for the V2 Miden "exact" payment scheme.
//!
//! This module defines the Miden-specific types used in the x402 protocol
//! wire format for payment requirements and error handling.

use serde::{Deserialize, Serialize};
use x402_types::proto::v2;

use crate::chain::MidenAccountAddress;

/// String literal for the "exact" scheme name.
#[derive(Debug, Clone, Copy)]
pub struct ExactScheme;

impl AsRef<str> for ExactScheme {
    fn as_ref(&self) -> &str {
        "exact"
    }
}

impl std::fmt::Display for ExactScheme {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "exact")
    }
}

impl Serialize for ExactScheme {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("exact")
    }
}

impl<'de> Deserialize<'de> for ExactScheme {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        if s == "exact" {
            Ok(ExactScheme)
        } else {
            Err(serde::de::Error::custom(format!(
                "expected 'exact', got '{s}'"
            )))
        }
    }
}

/// Type alias for V2 payment requirements with Miden-specific types.
///
/// Uses `ExactScheme` for the scheme name, `String` for amount (u64 as string),
/// `MidenAccountAddress` for addresses, and no extra data.
pub type PaymentRequirements =
    v2::PaymentRequirements<ExactScheme, String, MidenAccountAddress, Option<serde_json::Value>>;

/// Errors specific to Miden payment processing.
#[derive(Debug, thiserror::Error)]
pub enum MidenExactError {
    /// Invalid proof or verification failure.
    #[error("Invalid proof: {0}")]
    InvalidProof(String),

    /// The payment note was not found or does not match expectations.
    #[error("Payment not found in transaction outputs: {0}")]
    PaymentNotFound(String),

    /// Chain ID mismatch between payload and requirements.
    #[error("Chain ID mismatch: expected {expected}, got {got}")]
    ChainIdMismatch { expected: String, got: String },

    /// Recipient mismatch between payload and requirements.
    #[error("Recipient mismatch: expected {expected}, got {got}")]
    RecipientMismatch { expected: String, got: String },

    /// Scheme mismatch between payload and requirements.
    #[error("Scheme mismatch: expected {expected}, got {got}")]
    SchemeMismatch { expected: String, got: String },

    /// Asset/faucet mismatch between payload and requirements.
    #[error("Asset mismatch: expected {expected}, got {got}")]
    AssetMismatch { expected: String, got: String },

    /// The payment amount is insufficient.
    #[error("Insufficient payment: required {required}, got {got}")]
    InsufficientPayment { required: String, got: String },

    /// The payment context or transaction has expired.
    #[error("Transaction expired at block {0}")]
    TransactionExpired(u64),

    /// Failed to deserialize data.
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// An error from the Miden provider.
    #[error("Provider error: {0}")]
    ProviderError(String),

    // --- Lightweight verification errors (bobbinth's design, 0xMiden/node#1796) ---
    /// The note ID does not match the expected value computed from
    /// `hash(recipient_digest, asset_commitment)`.
    #[error("NoteId mismatch: expected {expected}, got {got}")]
    NoteIdMismatch { expected: String, got: String },

    /// The Merkle inclusion proof (SparseMerklePath) is invalid or does
    /// not verify against the block's note commitment root.
    #[error("Invalid inclusion proof: {0}")]
    InclusionProofInvalid(String),

    /// The block header for the specified block number could not be fetched
    /// from the Miden node, so the Merkle root is unavailable for verification.
    #[error("Block header not found for block {0}")]
    BlockHeaderNotFound(u32),

    /// The payment context has expired — the agent took too long to submit
    /// the transaction and send back the lightweight payment header.
    #[error("Payment context expired")]
    PaymentContextExpired,

    /// No payment context was found for the given recipient digest.
    /// The context may have already been consumed or was never created.
    #[error("Payment context not found: {0}")]
    PaymentContextNotFound(String),
}

impl From<MidenExactError> for x402_types::scheme::X402SchemeFacilitatorError {
    fn from(value: MidenExactError) -> Self {
        match value {
            MidenExactError::NoteIdMismatch { expected, got } => {
                x402_types::scheme::X402SchemeFacilitatorError::PaymentVerification(
                    x402_types::proto::PaymentVerificationError::InvalidFormat(format!(
                        "NoteId mismatch: expected {expected}, got {got}"
                    )),
                )
            }
            other => {
                x402_types::scheme::X402SchemeFacilitatorError::OnchainFailure(other.to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exact_scheme_display() {
        assert_eq!(ExactScheme.to_string(), "exact");
    }

    #[test]
    fn test_exact_scheme_serde() {
        let json = serde_json::to_string(&ExactScheme).unwrap();
        assert_eq!(json, "\"exact\"");
        let deserialized: ExactScheme = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.to_string(), "exact");
    }
}
