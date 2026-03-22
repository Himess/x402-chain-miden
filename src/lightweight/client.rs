//! Client-side (agent) lightweight payment flow.
//!
//! Implements the agent side of bobbinth's design from 0xMiden/node#1796.
//! Unlike the standard flow where the agent sends a full `ProvenTransaction`
//! (~100 KB) to the facilitator, the lightweight flow has the agent:
//!
//! 1. Receive `{recipient_digest, asset, note_tag}` from the 402 response
//! 2. Create a P2ID note using the `recipient_digest`
//! 3. STARK-prove and submit the transaction **directly** to the Miden network
//! 4. Call `sync_state()` to obtain a note inclusion proof
//! 5. Send `{note_id, block_num, inclusion_proof}` (~200 bytes) to the server
//!
//! # Feature gating
//!
//! - `miden-client-native`: Full implementation using `miden_client::Client`
//!   for transaction execution, proving, network submission, and state sync.
//! - Without `miden-client-native`: Trait definition only (no implementation),
//!   allowing downstream crates to provide their own implementation.

use super::types::{LightweightPaymentHeader, LightweightPaymentRequirement};

/// Trait for lightweight payment creation (agent side).
///
/// This trait handles the complete lightweight payment flow:
///
/// 1. Create P2ID note from `recipient_digest` + `asset`
/// 2. Execute the transaction locally in the Miden VM
/// 3. Generate STARK proof
/// 4. Submit the proven transaction directly to the Miden network
/// 5. Call `sync_state()` to get the note inclusion proof
/// 6. Return a lightweight payment header (~200 bytes)
///
/// The facilitator never sees the full transaction — only the compact
/// inclusion proof.
#[cfg(feature = "client")]
#[async_trait::async_trait]
pub trait LightweightPayerLike: Send + Sync {
    /// Returns the sender's account ID as a hex string.
    fn account_id(&self) -> String;

    /// Creates a P2ID payment, proves it, submits it to the network,
    /// and returns a lightweight payment header with the inclusion proof.
    ///
    /// This is the full agent-side flow per bobbinth's design:
    ///
    /// 1. Build P2ID note from `requirement.recipient_digest` + `requirement.asset`
    /// 2. Execute the transaction locally in the Miden VM
    /// 3. Generate STARK proof
    /// 4. Submit the proven transaction to the Miden node
    /// 5. Call `sync_state()` to get note inclusion in a block
    /// 6. Return `{note_id, block_num, inclusion_proof}`
    ///
    /// # Errors
    ///
    /// Returns an `X402Error` if any step fails:
    /// - Invalid account/faucet IDs
    /// - Transaction execution failure
    /// - STARK proving failure
    /// - Network submission failure
    /// - Sync failure (note not included in time)
    async fn create_and_submit_payment(
        &self,
        requirement: &LightweightPaymentRequirement,
    ) -> Result<LightweightPaymentHeader, x402_types::scheme::client::X402Error>;
}

// ============================================================================
// LightweightMidenPayer — real implementation using miden-client
// ============================================================================

/// A lightweight payer backed by a `miden_client::Client`.
///
/// This struct implements the full agent-side lightweight payment flow:
/// create P2ID note, prove, submit to network, sync, and return the
/// compact inclusion proof.
///
/// # Example
///
/// ```ignore
/// use x402_chain_miden::lightweight::client::LightweightMidenPayer;
/// use std::sync::Arc;
/// use tokio::sync::Mutex;
///
/// let payer = LightweightMidenPayer::new("0xsender...", client);
/// let header = payer.create_and_submit_payment(&requirement).await?;
/// // header.note_id, header.block_num, header.inclusion_proof
/// ```
#[cfg(feature = "miden-client-native")]
pub struct LightweightMidenPayer {
    account_id_hex: String,
    client: std::sync::Arc<
        tokio::sync::Mutex<miden_client::Client<miden_client::keystore::FilesystemKeyStore>>,
    >,
}

#[cfg(feature = "miden-client-native")]
impl LightweightMidenPayer {
    /// Creates a new lightweight payer.
    ///
    /// # Parameters
    ///
    /// - `account_id_hex`: The sender's Miden account ID (hex, with or without `0x` prefix).
    ///   The account must already exist in the client's store.
    /// - `client`: A shared reference to a `miden_client::Client`. The `Mutex`
    ///   ensures exclusive access during transaction execution and sync.
    pub fn new(
        account_id_hex: impl Into<String>,
        client: std::sync::Arc<
            tokio::sync::Mutex<miden_client::Client<miden_client::keystore::FilesystemKeyStore>>,
        >,
    ) -> Self {
        Self {
            account_id_hex: account_id_hex.into(),
            client,
        }
    }
}

#[cfg(feature = "miden-client-native")]
impl std::fmt::Debug for LightweightMidenPayer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LightweightMidenPayer")
            .field("account_id_hex", &self.account_id_hex)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "miden-client-native")]
impl Clone for LightweightMidenPayer {
    fn clone(&self) -> Self {
        Self {
            account_id_hex: self.account_id_hex.clone(),
            client: self.client.clone(),
        }
    }
}

#[cfg(feature = "miden-client-native")]
#[async_trait::async_trait]
impl LightweightPayerLike for LightweightMidenPayer {
    fn account_id(&self) -> String {
        self.account_id_hex.clone()
    }

    async fn create_and_submit_payment(
        &self,
        requirement: &LightweightPaymentRequirement,
    ) -> Result<LightweightPaymentHeader, x402_types::scheme::client::X402Error> {
        use miden_client::note::build_p2id_recipient;
        use miden_protocol::account::AccountId;
        use miden_protocol::asset::{Asset, FungibleAsset};
        use miden_protocol::note::{Note, NoteAssets, NoteMetadata, NoteTag, NoteType};
        use miden_protocol::transaction::OutputNote;
        use miden_protocol::utils::serde::Serializable;
        use miden_protocol::{Felt, Word};
        use x402_types::scheme::client::X402Error;

        // 1. Parse account IDs
        let sender = AccountId::from_hex(&self.account_id_hex)
            .map_err(|e| X402Error::SigningError(format!("Invalid sender account ID: {e}")))?;
        let target = AccountId::from_hex(&requirement.pay_to).map_err(|e| {
            X402Error::SigningError(format!("Invalid target account ID (pay_to): {e}"))
        })?;
        let faucet = AccountId::from_hex(&requirement.asset)
            .map_err(|e| X402Error::SigningError(format!("Invalid faucet account ID: {e}")))?;

        // 2. Parse server's serial_num from hex into Word ([Felt; 4])
        let serial_num_hex = requirement.serial_num.as_deref().ok_or_else(|| {
            X402Error::SigningError(
                "serial_num is required in LightweightPaymentRequirement for note construction"
                    .into(),
            )
        })?;
        let serial_bytes = hex::decode(serial_num_hex.strip_prefix("0x").unwrap_or(serial_num_hex))
            .map_err(|e| X402Error::SigningError(format!("Invalid serial_num hex: {e}")))?;
        if serial_bytes.len() != 32 {
            return Err(X402Error::SigningError(format!(
                "serial_num must be 32 bytes, got {}",
                serial_bytes.len()
            )));
        }
        // Convert 32 bytes into Word = [Felt; 4], each Felt from 8 bytes LE
        let serial_num: Word = [
            Felt::new(u64::from_le_bytes(serial_bytes[0..8].try_into().unwrap())),
            Felt::new(u64::from_le_bytes(serial_bytes[8..16].try_into().unwrap())),
            Felt::new(u64::from_le_bytes(serial_bytes[16..24].try_into().unwrap())),
            Felt::new(u64::from_le_bytes(serial_bytes[24..32].try_into().unwrap())),
        ];

        // 3. Build P2ID NoteRecipient with the server's serial_num
        //    This ensures the note's recipient_digest matches what the server expects.
        let recipient = build_p2id_recipient(target, serial_num)
            .map_err(|e| X402Error::SigningError(format!("Failed to build P2ID recipient: {e}")))?;

        // 4. Build the Note manually with the custom recipient
        let asset = FungibleAsset::new(faucet, requirement.amount)
            .map_err(|e| X402Error::SigningError(format!("Failed to create FungibleAsset: {e}")))?;
        let vault = NoteAssets::new(vec![Asset::Fungible(asset)])
            .map_err(|e| X402Error::SigningError(format!("Invalid note assets: {e}")))?;

        let tag = NoteTag::new(requirement.note_tag);
        let metadata = NoteMetadata::new(sender, NoteType::Private, tag);

        let note = Note::new(vault, metadata, recipient);
        let note_id_str = format!("{}", note.id());

        // 5. Build transaction request with our custom note (bypassing build_pay_to_id
        //    which would generate its own serial_num)
        let tx_request = miden_client::transaction::TransactionRequestBuilder::new()
            .own_output_notes(vec![OutputNote::Full(note)])
            .build()
            .map_err(|e| {
                X402Error::SigningError(format!("Failed to build TransactionRequest: {e}"))
            })?;

        // 6. Execute, prove, submit, and apply the transaction in one call.
        //    submit_new_transaction handles the full lifecycle:
        //      execute_transaction -> prove_transaction -> submit_proven_transaction -> apply_transaction
        let mut client_guard = self.client.lock().await;
        client_guard
            .submit_new_transaction(sender, tx_request)
            .await
            .map_err(|e| X402Error::SigningError(format!("Transaction submission failed: {e}")))?;

        // 7. Sync state to get the note inclusion proof from the network.
        //    After the transaction is committed to a block, sync_state will
        //    update the local store with inclusion proofs for output notes.
        client_guard
            .sync_state()
            .await
            .map_err(|e| X402Error::SigningError(format!("State sync failed: {e}")))?;

        // 8. Extract the inclusion proof from the client's output note store.
        //    After sync, committed notes have inclusion proofs attached.
        let output_notes = client_guard
            .get_output_notes(miden_client::store::NoteFilter::Committed)
            .await
            .map_err(|e| X402Error::SigningError(format!("Failed to query output notes: {e}")))?;

        let our_note = output_notes
            .iter()
            .find(|n| format!("{}", n.id()) == note_id_str)
            .ok_or_else(|| {
                X402Error::SigningError(
                    "Note not found in client store after sync — \
                     the transaction may not yet be committed to a block"
                        .into(),
                )
            })?;

        let inclusion_proof = our_note.inclusion_proof().ok_or_else(|| {
            X402Error::SigningError(
                "Note has no inclusion proof yet — may need additional sync cycles".into(),
            )
        })?;

        let block_num = inclusion_proof.location().block_num().as_u32();
        let note_index = inclusion_proof.location().node_index_in_block();
        let path_bytes = inclusion_proof.note_path().to_bytes();
        let path_hex = format!("0x{}", hex::encode(&path_bytes));
        let metadata_bytes = metadata.to_bytes();
        let metadata_hex = format!("0x{}", hex::encode(&metadata_bytes));

        drop(client_guard);

        Ok(LightweightPaymentHeader {
            note_id: note_id_str,
            block_num,
            note_index,
            note_metadata: metadata_hex,
            inclusion_proof: path_hex,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requirement_with_pay_to_and_serial_num() {
        // In production, serial_num is always provided by the server so the
        // agent can construct the note with the matching recipient_digest.
        let req = LightweightPaymentRequirement {
            recipient_digest: "0xdigest".to_string(),
            asset: "0x37d5977a8e16d8205a360820f0230f".to_string(),
            amount: 1_000_000,
            note_tag: 42,
            network: x402_types::chain::ChainId::new("miden", "testnet"),
            pay_to: "0xaabbccddeeff00112233aabbccddee".to_string(),
            serial_num: Some(
                "0x0102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f20".to_string(),
            ),
        };
        assert!(req.serial_num.is_some());
        assert_eq!(req.serial_num.as_deref().unwrap().len(), 66); // "0x" + 64 hex chars
    }

    #[test]
    fn test_requirement_serial_num_optional_at_type_level() {
        // The type keeps serial_num as Option<String> for backwards compatibility
        // (bobbinth's design says it's optional for privacy), but in practice
        // create_payment_requirement() always populates it.
        let req = LightweightPaymentRequirement {
            recipient_digest: "0xdigest".to_string(),
            asset: "0x37d5977a8e16d8205a360820f0230f".to_string(),
            amount: 1_000_000,
            note_tag: 42,
            network: x402_types::chain::ChainId::new("miden", "testnet"),
            pay_to: "0xaabbccddeeff00112233aabbccddee".to_string(),
            serial_num: None,
        };
        assert!(req.serial_num.is_none());
    }
}
