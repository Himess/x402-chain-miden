//! Type definitions for the V2 Miden "exact" payment scheme.
//!
//! This module defines the Miden-specific types used in the x402 protocol
//! wire format for payment authorization and verification.

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

/// The Miden-specific payment payload.
///
/// This contains the serialized proven transaction that the facilitator
/// can verify and submit to the Miden network.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MidenExactPayload {
    /// The sender's Miden account ID (hex-encoded).
    pub from: MidenAccountAddress,
    /// The serialized `ProvenTransaction` bytes (hex-encoded).
    ///
    /// This is the output of the client-side Miden VM execution and
    /// STARK proving process. It contains the full transaction proof
    /// that the facilitator can verify.
    pub proven_transaction: String,
    /// The transaction ID (hex-encoded hash of the proven transaction).
    pub transaction_id: String,
    /// The serialized `TransactionInputs` bytes (hex-encoded).
    ///
    /// Required for submitting the proven transaction to the Miden node.
    /// The node needs both the proven transaction and its inputs for
    /// mempool admission. Serialized using `miden_protocol::utils::serde::Serializable`.
    pub transaction_inputs: String,
}

/// Type alias for V2 payment requirements with Miden-specific types.
///
/// Uses `ExactScheme` for the scheme name, `String` for amount (u64 as string),
/// `MidenAccountAddress` for addresses, and no extra data.
pub type PaymentRequirements =
    v2::PaymentRequirements<ExactScheme, String, MidenAccountAddress, Option<serde_json::Value>>;

/// Type alias for V2 payment payloads with Miden-specific data.
pub type PaymentPayload = v2::PaymentPayload<PaymentRequirements, MidenExactPayload>;

/// Type alias for V2 verify requests.
pub type VerifyRequest = v2::VerifyRequest<PaymentPayload, PaymentRequirements>;

/// Type alias for V2 settle requests (same structure as verify).
pub type SettleRequest = VerifyRequest;

/// Errors specific to Miden payment processing.
#[derive(Debug, thiserror::Error)]
pub enum MidenExactError {
    /// The proven transaction is invalid or has an invalid proof.
    #[error("Invalid proof: {0}")]
    InvalidProof(String),

    /// The proven transaction's output notes do not contain the expected payment.
    #[error("Payment not found in transaction outputs: {0}")]
    PaymentNotFound(String),

    /// Chain ID mismatch between payload and requirements.
    #[error("Chain ID mismatch: expected {expected}, got {got}")]
    ChainIdMismatch { expected: String, got: String },

    /// Recipient mismatch between payload and requirements.
    #[error("Recipient mismatch: expected {expected}, got {got}")]
    RecipientMismatch { expected: String, got: String },

    /// The payment amount is insufficient.
    #[error("Insufficient payment: required {required}, got {got}")]
    InsufficientPayment { required: String, got: String },

    /// The transaction has expired.
    #[error("Transaction expired at block {0}")]
    TransactionExpired(u64),

    /// Failed to deserialize the proven transaction.
    #[error("Deserialization error: {0}")]
    DeserializationError(String),

    /// The accepted requirements don't match the provided requirements.
    #[error("Accepted requirements do not match provided requirements")]
    AcceptedRequirementsMismatch,

    /// An error from the Miden provider.
    #[error("Provider error: {0}")]
    ProviderError(String),
}

impl From<MidenExactError> for x402_types::scheme::X402SchemeFacilitatorError {
    fn from(value: MidenExactError) -> Self {
        match value {
            MidenExactError::InvalidProof(msg) => {
                x402_types::scheme::X402SchemeFacilitatorError::PaymentVerification(
                    x402_types::proto::PaymentVerificationError::InvalidFormat(msg),
                )
            }
            MidenExactError::AcceptedRequirementsMismatch => {
                x402_types::scheme::X402SchemeFacilitatorError::PaymentVerification(
                    x402_types::proto::PaymentVerificationError::InvalidFormat(
                        "Accepted requirements mismatch".to_string(),
                    ),
                )
            }
            other => x402_types::scheme::X402SchemeFacilitatorError::OnchainFailure(
                other.to_string(),
            ),
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

    #[test]
    fn test_miden_exact_payload_serde() {
        let payload = MidenExactPayload {
            from: "0xaabbccdd".parse().unwrap(),
            proven_transaction: "deadbeef".to_string(),
            transaction_id: "0x1234".to_string(),
            transaction_inputs: "cafebabe".to_string(),
        };
        let json = serde_json::to_string(&payload).unwrap();
        let deserialized: MidenExactPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.from, payload.from);
        assert_eq!(deserialized.proven_transaction, "deadbeef");
        assert_eq!(deserialized.transaction_id, "0x1234");
        assert_eq!(deserialized.transaction_inputs, "cafebabe");
    }
}
