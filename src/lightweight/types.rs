//! Type definitions for the lightweight payment verification scheme.
//!
//! This module defines the wire-format types used in bobbinth's lightweight
//! verification flow (0xMiden/node#1796). The types model the three phases
//! of the protocol:
//!
//! 1. **402 Response** ([`LightweightPaymentRequirement`]) — server tells the
//!    agent what to pay and how to tag the note.
//! 2. **Payment Header** ([`LightweightPaymentHeader`]) — agent sends back
//!    a compact proof of payment (~200 bytes).
//! 3. **Server State** ([`PaymentContext`]) — server-side bookkeeping for a
//!    pending payment, including the secret `serial_num`.

use serde::{Deserialize, Serialize};
use x402_types::chain::ChainId;

// ---------------------------------------------------------------------------
// LightweightPaymentRequirement — what the server sends in the 402 response
// ---------------------------------------------------------------------------

/// Payment requirement sent by the server in the HTTP 402 response body.
///
/// The server computes `recipient_digest` from:
/// - A freshly generated random `serial_num`
/// - The deterministic P2ID script root (from `miden-standards`)
/// - The recipient's account ID
///
/// Only the `recipient_digest` is exposed to the agent so the agent
/// can construct a P2ID note that satisfies the requirement. The
/// `serial_num` stays on the server unless `include_serial_num` was
/// set to `true` (for optional nullifier tracking).
///
/// # Wire format (JSON, camelCase)
///
/// ```json
/// {
///   "recipientDigest": "0xabcdef...",
///   "asset": "0x37d5977a8e16d8205a360820f0230f",
///   "amount": 1000000,
///   "noteTag": 12345,
///   "network": "miden:testnet",
///   "payTo": "0xaabbccddeeff...",
///   "serialNum": "0x0102030405..."
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightweightPaymentRequirement {
    /// The recipient digest (hex-encoded, 32 bytes).
    ///
    /// Computed as `hash(hash(serial_num, EMPTY), script_root, inputs_commitment)`
    /// where `inputs_commitment = hash(recipient_account_id)`.
    pub recipient_digest: String,

    /// The faucet (token) account ID (hex-encoded).
    ///
    /// Identifies which fungible asset the agent must include in the P2ID note.
    pub asset: String,

    /// The required payment amount in the token's smallest unit.
    pub amount: u64,

    /// The `NoteTag` value the agent must attach to the note.
    ///
    /// The server picks a tag that allows it to efficiently filter for
    /// incoming notes via `sync_state`.
    pub note_tag: u32,

    /// The CAIP-2 chain identifier (e.g. `miden:testnet`).
    pub network: ChainId,

    /// The recipient's Miden account ID (hex-encoded).
    ///
    /// The agent needs this to construct the P2ID note (the `recipient_digest`
    /// alone is not sufficient to build the note — the agent also needs to
    /// know the target account). This is the raw account ID, not the digest.
    pub pay_to: String,

    /// Hex-encoded serial number (32 bytes).
    ///
    /// The agent MUST use this serial_num when constructing the P2ID note
    /// so that the note's recipient_digest matches what the server expects.
    /// Without it, the agent would generate its own serial_num and the
    /// server's NoteId verification would fail.
    ///
    /// Kept as `Option<String>` at the type level for backwards compatibility
    /// (bobbinth's design allows omission for privacy), but in practice
    /// `create_payment_requirement()` always populates this field.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial_num: Option<String>,
}

// ---------------------------------------------------------------------------
// LightweightPaymentHeader — what the agent sends back
// ---------------------------------------------------------------------------

/// Compact payment proof sent by the agent after submitting the transaction.
///
/// Instead of sending the full `ProvenTransaction` (~100 KB), the agent
/// submits the transaction directly to the Miden network, waits for
/// inclusion, and sends only this lightweight header (~200 bytes).
///
/// # Wire format (JSON, camelCase)
///
/// ```json
/// {
///   "noteId": "0xdeadbeef...",
///   "blockNum": 42,
///   "noteIndex": 5,
///   "noteMetadata": "0xaabb...",
///   "inclusionProof": "0xcafebabe..."
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightweightPaymentHeader {
    /// The note ID (hex-encoded, 32 bytes).
    ///
    /// The server verifies this matches the expected note ID computed from
    /// `hash(recipient_digest, asset_commitment)`.
    pub note_id: String,

    /// The block number in which the note was included.
    ///
    /// The server fetches the block header for this block number to verify
    /// the inclusion proof against the block's note commitment root.
    pub block_num: u32,

    /// The note's index in the block's note tree (SparseMerkleTree).
    ///
    /// This is the `node_index_in_block` from the `NoteInclusionProof`
    /// returned by the Miden node after the note is included in a block.
    /// The index is computed as `batch_idx * MAX_OUTPUT_NOTES_PER_BATCH + note_idx_in_batch`.
    /// Needed for `SparseMerklePath::verify()`.
    ///
    /// Miden's note tree supports up to 2^16 notes per block (`SimpleSmt<16>`),
    /// so `u16` is sufficient.
    pub note_index: u16,

    /// The note metadata (hex-encoded serialized `NoteMetadata`).
    ///
    /// Contains the sender account ID, note type, note tag, and optional
    /// attachment. The server uses this together with the `note_id` to
    /// compute the note commitment (`hash(note_id || metadata_commitment)`)
    /// which is the leaf value in the block's note tree. Required for
    /// Merkle path verification.
    pub note_metadata: String,

    /// The Merkle inclusion proof (hex-encoded `SparseMerklePath`).
    ///
    /// Proves that the note is included in the note tree of the specified
    /// block. Verification is a sequence of O(log n) hash operations.
    pub inclusion_proof: String,
}

// ---------------------------------------------------------------------------
// PaymentContext — server-side state for a pending payment
// ---------------------------------------------------------------------------

/// Server-side state for a pending lightweight payment.
///
/// Created when the server issues a 402 response and stored until the
/// agent sends back a [`LightweightPaymentHeader`] or the context expires.
///
/// The server keeps the `serial_num` internally even when it is not
/// shared with the agent, because the serial number is needed to
/// recompute the expected `NoteId` during verification.
pub struct PaymentContext {
    /// The recipient digest that was sent to the agent (hex-encoded).
    pub recipient_digest: String,

    /// The faucet (token) account ID (hex-encoded).
    pub asset_faucet_id: String,

    /// The required payment amount in the token's smallest unit.
    pub amount: u64,

    /// The `NoteTag` value the agent was instructed to use.
    pub note_tag: u32,

    /// The serial number used to derive `recipient_digest` (hex-encoded).
    ///
    /// Always stored server-side; only optionally shared with the agent.
    pub serial_num: Option<String>,

    /// The expected note ID, computed lazily during verification.
    ///
    /// `NoteId = hash(recipient_digest, asset_commitment)` — set when
    /// the server first verifies a payment header against this context.
    pub expected_note_id: Option<String>,

    /// When this context was created, as a Unix timestamp (seconds since epoch).
    ///
    /// Using `u64` instead of `std::time::Instant` makes `PaymentContext`
    /// serializable and persistable across process restarts.
    pub created_at: u64,
}

impl PaymentContext {
    /// Creates a new payment context.
    ///
    /// `created_at` is set to the current time as seconds since the Unix epoch.
    ///
    /// # Parameters
    ///
    /// - `recipient_digest` — hex-encoded digest sent to the agent
    /// - `asset_faucet_id` — hex-encoded faucet account ID
    /// - `amount` — required payment in smallest token units
    /// - `note_tag` — the `NoteTag` value sent to the agent
    /// - `serial_num` — hex-encoded serial number (kept server-side)
    pub fn new(
        recipient_digest: String,
        asset_faucet_id: String,
        amount: u64,
        note_tag: u32,
        serial_num: Option<String>,
    ) -> Self {
        use std::time::{SystemTime, UNIX_EPOCH};
        let created_at = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_secs();
        Self {
            recipient_digest,
            asset_faucet_id,
            amount,
            note_tag,
            serial_num,
            expected_note_id: None,
            created_at,
        }
    }

    /// Returns `true` if this context has exceeded the given timeout.
    ///
    /// Expired contexts should be discarded — the agent took too long
    /// to submit the transaction and send back the payment header.
    pub fn is_expired(&self, timeout_secs: u64) -> bool {
        use std::time::{SystemTime, UNIX_EPOCH};
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock is before Unix epoch")
            .as_secs();
        now.saturating_sub(self.created_at) > timeout_secs
    }
}

// ---------------------------------------------------------------------------
// LightweightVerifyResponse — verification result
// ---------------------------------------------------------------------------

/// The result of lightweight payment verification.
///
/// Returned by the server after checking the [`LightweightPaymentHeader`]
/// against the [`PaymentContext`] and the block's note tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LightweightVerifyResponse {
    /// Whether the payment was verified successfully.
    pub valid: bool,

    /// The note ID that was verified (hex-encoded).
    pub note_id: String,

    /// The block number in which the note was found.
    pub block_num: u32,

    /// An error message if verification failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Parses a hex-encoded serial number (32 bytes) into a Miden `Word` (`[Felt; 4]`).
///
/// Each of the four `Felt` values is constructed from 8 little-endian bytes.
/// The input may optionally start with a `0x` prefix.
///
/// # Errors
///
/// Returns `Err` if the hex string is invalid or does not decode to exactly 32 bytes.
#[cfg(feature = "miden-native")]
pub fn parse_serial_num_hex(serial_num_hex: &str) -> Result<miden_protocol::Word, String> {
    use miden_protocol::{Felt, Word};

    let serial_bytes = hex::decode(serial_num_hex.strip_prefix("0x").unwrap_or(serial_num_hex))
        .map_err(|e| format!("Invalid serial_num hex: {e}"))?;
    if serial_bytes.len() != 32 {
        return Err(format!(
            "serial_num must be 32 bytes, got {}",
            serial_bytes.len()
        ));
    }
    Ok(Word::new([
        Felt::new(u64::from_le_bytes(serial_bytes[0..8].try_into().unwrap())),
        Felt::new(u64::from_le_bytes(serial_bytes[8..16].try_into().unwrap())),
        Felt::new(u64::from_le_bytes(serial_bytes[16..24].try_into().unwrap())),
        Felt::new(u64::from_le_bytes(serial_bytes[24..32].try_into().unwrap())),
    ]))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_payment_requirement_serde_roundtrip() {
        let req = LightweightPaymentRequirement {
            recipient_digest: "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                .to_string(),
            asset: "0x37d5977a8e16d8205a360820f0230f".to_string(),
            amount: 1_000_000,
            note_tag: 12345,
            network: ChainId::new("miden", "testnet"),
            pay_to: "0xaabbccddeeff00112233aabbccddee".to_string(),
            serial_num: None,
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"recipientDigest\""));
        assert!(json.contains("\"noteTag\""));
        assert!(!json.contains("\"serialNum\""));

        let deserialized: LightweightPaymentRequirement = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.recipient_digest, req.recipient_digest);
        assert_eq!(deserialized.asset, req.asset);
        assert_eq!(deserialized.amount, req.amount);
        assert_eq!(deserialized.note_tag, req.note_tag);
        assert_eq!(deserialized.network.to_string(), "miden:testnet");
        assert!(deserialized.serial_num.is_none());
    }

    #[test]
    fn test_payment_requirement_serde_with_serial_num() {
        let req = LightweightPaymentRequirement {
            recipient_digest: "0xabcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890"
                .to_string(),
            asset: "0x37d5977a8e16d8205a360820f0230f".to_string(),
            amount: 500_000,
            note_tag: 99,
            network: ChainId::new("miden", "mainnet"),
            pay_to: "0xaabbccddeeff00112233aabbccddee".to_string(),
            serial_num: Some(
                "0x1111111122222222333333334444444455555555666666667777777788888888".to_string(),
            ),
        };
        let json = serde_json::to_string(&req).unwrap();
        assert!(json.contains("\"serialNum\""));

        let deserialized: LightweightPaymentRequirement = serde_json::from_str(&json).unwrap();
        assert!(deserialized.serial_num.is_some());
        assert_eq!(
            deserialized.serial_num.as_deref(),
            Some("0x1111111122222222333333334444444455555555666666667777777788888888")
        );
    }

    #[test]
    fn test_payment_header_serde_roundtrip() {
        let header = LightweightPaymentHeader {
            note_id: "0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef"
                .to_string(),
            block_num: 42,
            note_index: 5,
            note_metadata: "0xaabbccdd".to_string(),
            inclusion_proof: "0xcafebabe".to_string(),
        };
        let json = serde_json::to_string(&header).unwrap();
        assert!(json.contains("\"noteId\""));
        assert!(json.contains("\"blockNum\""));
        assert!(json.contains("\"noteIndex\""));
        assert!(json.contains("\"noteMetadata\""));
        assert!(json.contains("\"inclusionProof\""));

        let deserialized: LightweightPaymentHeader = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.note_id, header.note_id);
        assert_eq!(deserialized.block_num, header.block_num);
        assert_eq!(deserialized.note_index, header.note_index);
        assert_eq!(deserialized.note_metadata, header.note_metadata);
        assert_eq!(deserialized.inclusion_proof, header.inclusion_proof);
    }

    #[test]
    fn test_verify_response_serde_roundtrip_valid() {
        let resp = LightweightVerifyResponse {
            valid: true,
            note_id: "0xabcd".to_string(),
            block_num: 100,
            error: None,
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(!json.contains("\"error\""));

        let deserialized: LightweightVerifyResponse = serde_json::from_str(&json).unwrap();
        assert!(deserialized.valid);
        assert_eq!(deserialized.note_id, "0xabcd");
        assert_eq!(deserialized.block_num, 100);
        assert!(deserialized.error.is_none());
    }

    #[test]
    fn test_verify_response_serde_roundtrip_invalid() {
        let resp = LightweightVerifyResponse {
            valid: false,
            note_id: "0xabcd".to_string(),
            block_num: 100,
            error: Some("NoteId mismatch".to_string()),
        };
        let json = serde_json::to_string(&resp).unwrap();
        assert!(json.contains("\"error\""));

        let deserialized: LightweightVerifyResponse = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.valid);
        assert_eq!(deserialized.error.as_deref(), Some("NoteId mismatch"));
    }

    #[test]
    fn test_payment_context_new() {
        let ctx = PaymentContext::new(
            "0xaabb".to_string(),
            "0xccdd".to_string(),
            1_000_000,
            42,
            Some("0xserial".to_string()),
        );
        assert_eq!(ctx.recipient_digest, "0xaabb");
        assert_eq!(ctx.asset_faucet_id, "0xccdd");
        assert_eq!(ctx.amount, 1_000_000);
        assert_eq!(ctx.note_tag, 42);
        assert_eq!(ctx.serial_num.as_deref(), Some("0xserial"));
        assert!(ctx.expected_note_id.is_none());
    }

    #[test]
    fn test_payment_context_is_expired() {
        let ctx = PaymentContext::new(
            "0xaabb".to_string(),
            "0xccdd".to_string(),
            1_000_000,
            42,
            None,
        );
        // Just created — should not be expired with a 60-second timeout
        assert!(!ctx.is_expired(60));
        // With a 0-second timeout, anything is expired
        assert!(ctx.is_expired(0));
    }

    #[test]
    fn test_payment_requirement_deserialize_missing_serial_num() {
        let json = r#"{
            "recipientDigest": "0xaabb",
            "asset": "0xccdd",
            "amount": 100,
            "noteTag": 1,
            "network": "miden:testnet",
            "payTo": "0xaabbccddeeff00112233aabbccddee"
        }"#;
        let req: LightweightPaymentRequirement = serde_json::from_str(json).unwrap();
        assert!(req.serial_num.is_none());
    }

    #[test]
    fn test_payment_header_json_camel_case_keys() {
        let header = LightweightPaymentHeader {
            note_id: "0xaa".to_string(),
            block_num: 1,
            note_index: 0,
            note_metadata: "0xcc".to_string(),
            inclusion_proof: "0xbb".to_string(),
        };
        let json = serde_json::to_string(&header).unwrap();
        // Verify camelCase keys (not snake_case)
        assert!(json.contains("\"noteId\""));
        assert!(json.contains("\"blockNum\""));
        assert!(json.contains("\"noteIndex\""));
        assert!(json.contains("\"noteMetadata\""));
        assert!(json.contains("\"inclusionProof\""));
        assert!(!json.contains("\"note_id\""));
        assert!(!json.contains("\"block_num\""));
        assert!(!json.contains("\"note_index\""));
        assert!(!json.contains("\"note_metadata\""));
        assert!(!json.contains("\"inclusion_proof\""));
    }
}
