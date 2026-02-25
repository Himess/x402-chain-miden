//! Client-side payment signing for the V2 Miden "exact" scheme.
//!
//! This module provides [`V2MidenExactClient`] for creating and signing
//! P2ID note payments on the Miden blockchain using the V2 protocol.
//!
//! # Payment Flow
//!
//! 1. Client receives 402 response with Miden payment requirements
//! 2. `V2MidenExactClient::accept()` generates payment candidates
//! 3. For each candidate, `MidenPayloadSigner::sign_payment()`:
//!    a. Creates a P2ID note (sender → recipient)
//!    b. Executes the transaction in Miden VM
//!    c. Generates STARK proof
//!    d. Serializes the ProvenTransaction as the payload
//! 4. The base64-encoded payload is sent as the `Payment-Signature` header

use async_trait::async_trait;
use x402_types::proto::v2::ResourceInfo;
use x402_types::proto::{OriginalJson, PaymentRequired, v2};
use x402_types::scheme::X402SchemeId;
use x402_types::scheme::client::{
    PaymentCandidate, PaymentCandidateSigner, X402Error, X402SchemeClient,
};
use x402_types::util::Base64Bytes;

use crate::chain::MidenChainReference;
use crate::privacy::PrivacyMode;
use crate::v2_miden_exact::V2MidenExact;
use crate::v2_miden_exact::types::{self, MidenExactPayload};

/// Trait for Miden transaction signing.
///
/// Implementations of this trait handle the creation of P2ID notes,
/// transaction execution, proving, and serialization.
#[async_trait]
pub trait MidenSignerLike: Send + Sync {
    /// Returns the sender's Miden account ID as a hex string.
    fn account_id(&self) -> String;

    /// Creates a P2ID payment, proves it, and returns the serialized proven transaction.
    ///
    /// # Parameters
    ///
    /// - `recipient`: The recipient's Miden account ID (hex)
    /// - `faucet_id`: The faucet account ID for the token (hex)
    /// - `amount`: The amount in the token's smallest unit
    ///
    /// # Returns
    ///
    /// A tuple of `(proven_transaction_hex, transaction_id_hex, transaction_inputs_hex)`.
    /// The `transaction_inputs_hex` is needed by the facilitator to submit the
    /// proven transaction to the Miden node.
    async fn create_and_prove_p2id(
        &self,
        recipient: &str,
        faucet_id: &str,
        amount: u64,
    ) -> Result<(String, String, String), X402Error>;

    /// Creates a P2ID payment with a specific privacy mode, proves it, and returns
    /// the serialized proven transaction plus optional off-chain note data.
    ///
    /// # Returns
    ///
    /// A tuple of `(proven_transaction_hex, transaction_id_hex, transaction_inputs_hex, note_data_hex)`.
    /// `note_data_hex` is `Some` when `privacy_mode` is `TrustedFacilitator` (the full note
    /// must be shared off-chain with the facilitator).
    ///
    /// The default implementation delegates to [`create_and_prove_p2id`](Self::create_and_prove_p2id)
    /// for `Public` mode and returns an error for other modes.
    async fn create_and_prove_p2id_with_privacy(
        &self,
        recipient: &str,
        faucet_id: &str,
        amount: u64,
        privacy_mode: &PrivacyMode,
    ) -> Result<(String, String, String, Option<String>), X402Error> {
        match privacy_mode {
            PrivacyMode::Public => {
                let (tx, id, inputs) =
                    self.create_and_prove_p2id(recipient, faucet_id, amount).await?;
                Ok((tx, id, inputs, None))
            }
            other => Err(X402Error::SigningError(format!(
                "Privacy mode '{other}' requires miden-client-native feature"
            ))),
        }
    }
}

/// Client for signing V2 Miden exact scheme payments.
///
/// This client handles the creation and proving of P2ID note payments
/// for the Miden blockchain using the V2 protocol.
///
/// # Type Parameters
///
/// - `S`: The signer type, which must implement [`MidenSignerLike`]
///
/// # Example
///
/// ```ignore
/// use x402_chain_miden::V2MidenExactClient;
///
/// let client = V2MidenExactClient::new(miden_signer);
/// let candidates = client.accept(&payment_required);
/// ```
#[derive(Debug)]
pub struct V2MidenExactClient<S> {
    signer: S,
    privacy_mode: PrivacyMode,
}

impl<S> V2MidenExactClient<S> {
    /// Creates a new V2 Miden exact scheme client with the given signer.
    ///
    /// Uses `PrivacyMode::Public` by default for backward compatibility.
    pub fn new(signer: S) -> Self {
        Self {
            signer,
            privacy_mode: PrivacyMode::Public,
        }
    }

    /// Creates a new V2 Miden exact scheme client with a specific privacy mode.
    pub fn with_privacy_mode(signer: S, privacy_mode: PrivacyMode) -> Self {
        Self {
            signer,
            privacy_mode,
        }
    }
}

impl<S> X402SchemeId for V2MidenExactClient<S> {
    fn namespace(&self) -> &str {
        V2MidenExact.namespace()
    }

    fn scheme(&self) -> &str {
        V2MidenExact.scheme()
    }
}

impl<S> X402SchemeClient for V2MidenExactClient<S>
where
    S: MidenSignerLike + Clone + Send + Sync + 'static,
{
    fn accept(&self, payment_required: &PaymentRequired) -> Vec<PaymentCandidate> {
        let payment_required = match payment_required {
            PaymentRequired::V2(payment_required) => payment_required,
            PaymentRequired::V1(_) => {
                return vec![];
            }
        };
        payment_required
            .accepts
            .iter()
            .filter_map(|original_requirements_json| {
                let requirements =
                    types::PaymentRequirements::try_from(original_requirements_json).ok()?;
                let _chain_reference =
                    MidenChainReference::try_from(&requirements.network).ok()?;

                // Parse amount from string to u64 for the candidate
                let amount_str = &requirements.amount;
                let amount_u64: u64 = amount_str.parse().ok()?;

                let candidate = PaymentCandidate {
                    chain_id: requirements.network.clone(),
                    asset: requirements.asset.to_string(),
                    amount: alloy_primitives::U256::from(amount_u64),
                    scheme: self.scheme().to_string(),
                    x402_version: self.x402_version(),
                    pay_to: requirements.pay_to.to_string(),
                    signer: Box::new(MidenPayloadSigner {
                        resource_info: Some(payment_required.resource.clone()),
                        signer: self.signer.clone(),
                        privacy_mode: self.privacy_mode,
                        requirements,
                        requirements_json: original_requirements_json.clone(),
                    }),
                };
                Some(candidate)
            })
            .collect::<Vec<_>>()
    }
}

// ============================================================================
// MidenClientSigner — real signer using miden-client
// ============================================================================

/// A signer backed by a `miden_client::Client` that creates P2ID notes,
/// executes them in the Miden VM, generates STARK proofs, and serializes
/// the resulting `ProvenTransaction`.
///
/// This requires the `miden-client-native` feature flag.
///
/// # Example
///
/// ```ignore
/// use x402_chain_miden::v2_miden_exact::client::MidenClientSigner;
///
/// let signer = MidenClientSigner::new(client, sender_account_id);
/// let x402_client = V2MidenExactClient::new(signer);
/// ```
#[cfg(feature = "miden-client-native")]
pub struct MidenClientSigner {
    account_id_hex: String,
    client: std::sync::Arc<tokio::sync::Mutex<miden_client::Client<miden_client::keystore::FilesystemKeyStore>>>,
}

#[cfg(feature = "miden-client-native")]
impl MidenClientSigner {
    /// Creates a new signer backed by a `miden_client::Client`.
    ///
    /// The `account_id_hex` should be the hex-encoded Miden account ID
    /// (with or without `0x` prefix) of the sender account. The sender
    /// account must already exist in the client's store.
    pub fn new(
        account_id_hex: impl Into<String>,
        client: std::sync::Arc<tokio::sync::Mutex<miden_client::Client<miden_client::keystore::FilesystemKeyStore>>>,
    ) -> Self {
        Self {
            account_id_hex: account_id_hex.into(),
            client,
        }
    }
}

#[cfg(feature = "miden-client-native")]
impl std::fmt::Debug for MidenClientSigner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MidenClientSigner")
            .field("account_id_hex", &self.account_id_hex)
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "miden-client-native")]
impl Clone for MidenClientSigner {
    fn clone(&self) -> Self {
        Self {
            account_id_hex: self.account_id_hex.clone(),
            client: self.client.clone(),
        }
    }
}

#[cfg(feature = "miden-client-native")]
#[async_trait]
impl MidenSignerLike for MidenClientSigner {
    fn account_id(&self) -> String {
        self.account_id_hex.clone()
    }

    async fn create_and_prove_p2id(
        &self,
        recipient: &str,
        faucet_id: &str,
        amount: u64,
    ) -> Result<(String, String, String), X402Error> {
        let (tx, id, inputs, _) = self
            .create_and_prove_p2id_with_privacy(recipient, faucet_id, amount, &PrivacyMode::Public)
            .await?;
        Ok((tx, id, inputs))
    }

    async fn create_and_prove_p2id_with_privacy(
        &self,
        recipient: &str,
        faucet_id: &str,
        amount: u64,
        privacy_mode: &PrivacyMode,
    ) -> Result<(String, String, String, Option<String>), X402Error> {
        use miden_protocol::account::AccountId;
        use miden_protocol::asset::{Asset, FungibleAsset};
        use miden_protocol::note::NoteType;
        use miden_protocol::transaction::{OutputNote, TransactionInputs};
        use miden_protocol::utils::serde::Serializable;

        let note_type = match privacy_mode {
            PrivacyMode::Public => NoteType::Public,
            PrivacyMode::TrustedFacilitator => NoteType::Private,
        };

        // 1. Parse account IDs
        let sender = AccountId::from_hex(&self.account_id_hex).map_err(|e| {
            X402Error::SigningError(format!("Invalid sender account ID: {e}"))
        })?;
        let target = AccountId::from_hex(recipient).map_err(|e| {
            X402Error::SigningError(format!("Invalid recipient account ID: {e}"))
        })?;
        let faucet = AccountId::from_hex(faucet_id).map_err(|e| {
            X402Error::SigningError(format!("Invalid faucet ID: {e}"))
        })?;

        // 2. Create the fungible asset
        let asset = FungibleAsset::new(faucet, amount).map_err(|e| {
            X402Error::SigningError(format!("Failed to create FungibleAsset: {e}"))
        })?;

        // 3. Build a P2ID TransactionRequest via the builder.
        let mut client_guard = self.client.lock().await;

        let payment_data = miden_client::transaction::PaymentNoteDescription::new(
            vec![Asset::Fungible(asset)],
            sender,
            target,
        );

        let tx_request = miden_client::transaction::TransactionRequestBuilder::new()
            .build_pay_to_id(
                payment_data,
                note_type,
                client_guard.rng(),
            )
            .map_err(|e| {
                X402Error::SigningError(format!("Failed to build P2ID TransactionRequest: {e}"))
            })?;

        // 4. Execute the transaction locally in the Miden VM
        let tx_result = client_guard
            .execute_transaction(sender, tx_request)
            .await
            .map_err(|e| {
                X402Error::SigningError(format!("Transaction execution failed: {e}"))
            })?;

        // 5. For TrustedFacilitator mode, extract full note data BEFORE proving.
        //    The prover shrinks Private OutputNote::Full → OutputNote::Header,
        //    so this is the only opportunity to capture the full note.
        let note_data = if matches!(privacy_mode, PrivacyMode::TrustedFacilitator) {
            let full_note = tx_result
                .created_notes()
                .iter()
                .find_map(|on| {
                    if let OutputNote::Full(note) = on {
                        Some(note.clone())
                    } else {
                        None
                    }
                })
                .ok_or_else(|| {
                    X402Error::SigningError(
                        "No full note found in transaction result".to_string(),
                    )
                })?;
            Some(hex::encode(full_note.to_bytes()))
        } else {
            None
        };

        // 6. Extract TransactionInputs before proving.
        //    The facilitator needs these to submit the proven transaction
        //    to the Miden node (NodeRpcClient::submit_proven_transaction
        //    requires both ProvenTransaction and TransactionInputs).
        let tx_inputs = TransactionInputs::from(&tx_result);
        let tx_inputs_hex = hex::encode(tx_inputs.to_bytes());

        // 7. Generate STARK proof.
        //    Grab the prover (Arc<dyn TransactionProver + Send + Sync>)
        //    from the client, release the lock, then prove independently.
        let prover = client_guard.prover();
        drop(client_guard);

        let proven_tx = prover
            .prove(tx_result.into())
            .await
            .map_err(|e| X402Error::SigningError(format!("Transaction proving failed: {e}")))?;

        // 8. Serialize the ProvenTransaction — the facilitator will verify
        //    the proof and submit it to the network.
        let tx_bytes = proven_tx.to_bytes();
        let tx_hex = hex::encode(&tx_bytes);
        let tx_id = format!("{}", proven_tx.id());

        Ok((tx_hex, tx_id, tx_inputs_hex, note_data))
    }
}

/// Internal signer that creates and proves Miden P2ID payments.
struct MidenPayloadSigner<S> {
    signer: S,
    privacy_mode: PrivacyMode,
    resource_info: Option<ResourceInfo>,
    requirements: types::PaymentRequirements,
    requirements_json: OriginalJson,
}

#[async_trait]
impl<S> PaymentCandidateSigner for MidenPayloadSigner<S>
where
    S: MidenSignerLike + Sync,
{
    async fn sign_payment(&self) -> Result<String, X402Error> {
        let recipient = self.requirements.pay_to.to_string();
        let faucet_id = self.requirements.asset.to_string();
        let amount: u64 = self
            .requirements
            .amount
            .parse()
            .map_err(|_| X402Error::ParseError("Invalid amount".to_string()))?;

        // Create P2ID note, execute, prove (with privacy mode)
        let (proven_tx_hex, tx_id, tx_inputs_hex, note_data) = self
            .signer
            .create_and_prove_p2id_with_privacy(
                &recipient,
                &faucet_id,
                amount,
                &self.privacy_mode,
            )
            .await?;

        let miden_payload = MidenExactPayload {
            from: self
                .signer
                .account_id()
                .parse()
                .map_err(|e: crate::chain::MidenAddressParseError| {
                    X402Error::SigningError(e.to_string())
                })?,
            proven_transaction: proven_tx_hex,
            transaction_id: tx_id,
            transaction_inputs: tx_inputs_hex,
            privacy_mode: self.privacy_mode,
            note_data,
        };

        let payload = v2::PaymentPayload {
            x402_version: v2::X402Version2,
            accepted: self.requirements_json.clone(),
            resource: self.resource_info.clone(),
            payload: miden_payload,
        };

        let json = serde_json::to_vec(&payload)?;
        let b64 = Base64Bytes::encode(&json);

        Ok(b64.to_string())
    }
}
