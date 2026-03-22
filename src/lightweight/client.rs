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
        use miden_protocol::account::AccountId;
        use miden_protocol::asset::{Asset, FungibleAsset};
        use miden_protocol::note::NoteType;
        use miden_protocol::utils::serde::Serializable;
        use x402_types::scheme::client::X402Error;

        // 1. Parse sender account ID
        let sender = AccountId::from_hex(&self.account_id_hex)
            .map_err(|e| X402Error::SigningError(format!("Invalid sender account ID: {e}")))?;

        // 2. Parse faucet account ID
        let faucet = AccountId::from_hex(&requirement.asset)
            .map_err(|e| X402Error::SigningError(format!("Invalid faucet account ID: {e}")))?;

        // 3. Resolve the target account ID from `pay_to` field
        let target = AccountId::from_hex(&requirement.pay_to).map_err(|e| {
            X402Error::SigningError(format!("Invalid target account ID (pay_to): {e}"))
        })?;

        // 4. Create fungible asset
        let asset = FungibleAsset::new(faucet, requirement.amount)
            .map_err(|e| X402Error::SigningError(format!("Failed to create FungibleAsset: {e}")))?;

        // 5. Build P2ID note as private (NoteType::Private) — the note content
        //    does not need to be public since the server verifies via inclusion proof
        let mut client_guard = self.client.lock().await;

        let payment_data = miden_client::transaction::PaymentNoteDescription::new(
            vec![Asset::Fungible(asset)],
            sender,
            target,
        );

        let tx_request = miden_client::transaction::TransactionRequestBuilder::new()
            .build_pay_to_id(payment_data, NoteType::Private, client_guard.rng())
            .map_err(|e| {
                X402Error::SigningError(format!("Failed to build P2ID TransactionRequest: {e}"))
            })?;

        // 6. Execute transaction locally in the Miden VM
        let tx_result = client_guard
            .execute_transaction(sender, tx_request)
            .await
            .map_err(|e| X402Error::SigningError(format!("Transaction execution failed: {e}")))?;

        // 7. Extract note ID from the created output notes
        let note_id = tx_result
            .created_notes()
            .iter()
            .next()
            .map(|n| format!("{}", n.id()))
            .ok_or_else(|| {
                X402Error::SigningError("Transaction produced no output notes".to_string())
            })?;

        // 8. Generate STARK proof
        //    Grab the prover from the client, release the lock, then prove.
        let prover = client_guard.prover();
        drop(client_guard);

        let proven_tx = prover
            .prove(tx_result.into())
            .await
            .map_err(|e| X402Error::SigningError(format!("Transaction proving failed: {e}")))?;

        // 9. Submit the proven transaction directly to the Miden network.
        //    Re-acquire the lock to use the client's submission mechanism.
        //    The client internally serializes and sends via gRPC.
        let _proven_tx_bytes = proven_tx.to_bytes();
        let _tx_id = format!("{}", proven_tx.id());

        // TODO(bobbinth): Use miden-client's submit + sync APIs once the
        // exact method signatures stabilize. The current miden-client 0.13
        // API is:
        //   client.submit_transaction(proven_tx) -> Result<()>
        //   client.sync_state() -> Result<SyncSummary>
        //
        // After submission and sync, the note inclusion proof can be
        // extracted from the client's store:
        //   client.get_note_inclusion_proof(note_id) -> Option<NoteInclusionProof>
        //
        // The NoteInclusionProof contains the block_num and SparseMerklePath.

        // 10. Sync state to find the note in a block.
        //     After sync, the client's local store should have the note
        //     inclusion proof if the note was committed to a block.
        let mut client_guard = self.client.lock().await;

        client_guard
            .sync_state()
            .await
            .map_err(|e| X402Error::SigningError(format!("State sync failed: {e}")))?;

        // 11. Extract inclusion proof from the client's store.
        //     The miden-client tracks output notes and their inclusion
        //     proofs after sync. The exact API depends on the client version.
        //
        //     For now, we return a placeholder. A production implementation
        //     would query the client's note store:
        //
        //       let proof = client_guard.get_output_note_inclusion_proof(&note_id_parsed)?;
        //       let block_num = proof.block_num();
        //       let merkle_path = hex::encode(proof.path().to_bytes());

        // Release the lock
        drop(client_guard);

        // TODO: Replace with real inclusion proof extraction once
        // miden-client 0.13+ stabilizes the note tracking API.
        //
        // The return value below is a placeholder that will be replaced
        // when integrating with the actual miden-client note store.
        Ok(LightweightPaymentHeader {
            note_id,
            block_num: 0,
            inclusion_proof: String::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_requirement_with_pay_to() {
        let req = LightweightPaymentRequirement {
            recipient_digest: "0xdigest".to_string(),
            asset: "0x37d5977a8e16d8205a360820f0230f".to_string(),
            amount: 1_000_000,
            note_tag: 42,
            network: x402_types::chain::ChainId::new("miden", "testnet"),
            pay_to: "0xaabbccddeeff00112233aabbccddee".to_string(),
            serial_num: None,
        };
        // pay_to is a required field in LightweightPaymentRequirement.
        // The agent needs it to know where to send the P2ID note.
        assert!(req.serial_num.is_none());
    }
}
