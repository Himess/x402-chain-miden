//! Facilitator-side payment verification and settlement for V2 Miden exact scheme.
//!
//! This module implements the facilitator logic for V2 protocol payments on the
//! Miden blockchain. The facilitator:
//!
//! 1. **Verify**: Parses the payment payload, validates the STARK proof,
//!    checks that output notes contain the expected P2ID payment
//! 2. **Settle**: Submits the proven transaction to the Miden network

use std::collections::HashMap;
use x402_types::chain::ChainProviderOps;
use x402_types::proto;
use x402_types::proto::v2;
use x402_types::scheme::{
    X402SchemeFacilitator, X402SchemeFacilitatorBuilder, X402SchemeFacilitatorError,
};

use crate::V2MidenExact;
use crate::chain::MidenChainProvider;
use crate::v2_miden_exact::types::{self, ExactScheme, MidenExactError};

impl X402SchemeFacilitatorBuilder<MidenChainProvider> for V2MidenExact {
    fn build(
        &self,
        provider: MidenChainProvider,
        _config: Option<serde_json::Value>,
    ) -> Result<Box<dyn X402SchemeFacilitator>, Box<dyn std::error::Error>> {
        Ok(Box::new(V2MidenExactFacilitator::new(provider)))
    }
}

/// Facilitator for V2 Miden exact scheme payments.
///
/// This struct implements the [`X402SchemeFacilitator`] trait to provide payment
/// verification and settlement services for P2ID note-based payments on the
/// Miden blockchain.
pub struct V2MidenExactFacilitator {
    provider: MidenChainProvider,
}

impl V2MidenExactFacilitator {
    /// Creates a new V2 Miden exact scheme facilitator with the given provider.
    pub fn new(provider: MidenChainProvider) -> Self {
        Self { provider }
    }
}

#[async_trait::async_trait]
impl X402SchemeFacilitator for V2MidenExactFacilitator {
    async fn verify(
        &self,
        request: &proto::VerifyRequest,
    ) -> Result<proto::VerifyResponse, X402SchemeFacilitatorError> {
        let verify_request = types::FacilitatorVerifyRequest::try_from(request.clone())?;
        let verify_response = verify_miden_payment(&verify_request).await?;
        Ok(verify_response.into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let settle_request = types::FacilitatorSettleRequest::try_from(request.clone())?;
        let settle_response = settle_miden_payment(&self.provider, &settle_request).await?;
        Ok(settle_response.into())
    }

    async fn supported(&self) -> Result<proto::SupportedResponse, X402SchemeFacilitatorError> {
        let chain_id = self.provider.chain_id();
        let kinds = vec![proto::SupportedPaymentKind {
            x402_version: v2::X402Version2.into(),
            scheme: ExactScheme.to_string(),
            network: chain_id.clone().into(),
            extra: None,
        }];
        let signers = {
            let mut signers = HashMap::with_capacity(1);
            signers.insert(chain_id, self.provider.signer_addresses());
            signers
        };
        Ok(proto::SupportedResponse {
            kinds,
            extensions: Vec::new(),
            signers,
        })
    }
}

/// Checks that accepted requirements match provided requirements.
fn check_requirements_match(
    payload: &types::PaymentPayload,
    requirements: &types::PaymentRequirements,
) -> Result<(), MidenExactError> {
    let accepted = &payload.accepted;
    if accepted.network != requirements.network {
        return Err(MidenExactError::ChainIdMismatch {
            expected: requirements.network.to_string(),
            got: accepted.network.to_string(),
        });
    }
    if accepted.pay_to != requirements.pay_to {
        return Err(MidenExactError::RecipientMismatch {
            expected: requirements.pay_to.to_string(),
            got: accepted.pay_to.to_string(),
        });
    }
    Ok(())
}

/// Verifies a Miden payment payload using real STARK proof verification.
///
/// This implementation:
/// 1. Checks that the accepted requirements match the provided requirements
/// 2. Deserializes the `ProvenTransaction` from the hex payload
/// 3. Verifies the STARK proof using `TransactionVerifier`
/// 4. Checks that the output notes contain a P2ID payment to the correct recipient
///    with the correct faucet and amount
/// 5. Returns the verified payer account ID
#[cfg(feature = "miden-native")]
async fn verify_miden_payment(
    request: &types::FacilitatorVerifyRequest,
) -> Result<v2::VerifyResponse, MidenExactError> {
    use crate::chain::MidenAccountAddress;
    use miden_protocol::transaction::{OutputNote, ProvenTransaction};
    use miden_protocol::utils::serde::Deserializable;
    use miden_standards::note::P2idNoteStorage;
    use miden_tx::TransactionVerifier;

    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    check_requirements_match(payload, requirements)?;

    let miden_payload = &payload.payload;

    // 1. Decode hex → bytes
    let proven_tx_bytes = hex::decode(&miden_payload.proven_transaction).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in proven_transaction: {e}"))
    })?;

    // 2. Deserialize ProvenTransaction
    let proven_tx = ProvenTransaction::read_from_bytes(&proven_tx_bytes).map_err(|e| {
        MidenExactError::DeserializationError(format!("Failed to deserialize ProvenTransaction: {e}"))
    })?;

    // 3. Verify STARK proof (security level 96 = standard)
    let verifier = TransactionVerifier::new(96);
    verifier.verify(&proven_tx).map_err(|e| {
        MidenExactError::InvalidProof(format!("STARK proof verification failed: {e}"))
    })?;

    // 4. Check output notes for P2ID payment to correct recipient with correct amount
    let required_recipient = requirements.pay_to.to_account_id().map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid pay_to account ID: {e}"))
    })?;

    let required_faucet = requirements.asset.to_account_id().map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid asset/faucet account ID: {e}"))
    })?;

    let required_amount: u64 = requirements
        .amount
        .parse()
        .map_err(|_| MidenExactError::DeserializationError("Invalid amount".to_string()))?;

    let mut payment_found = false;

    for output_note in proven_tx.output_notes().iter() {
        // Only Full output notes can be inspected (public notes)
        if let OutputNote::Full(note) = output_note {
            // Check if this is a P2ID note by comparing script roots
            let script_root = note.recipient().script().root();
            if script_root != miden_standards::note::P2idNote::script_root() {
                continue;
            }

            // Parse P2ID storage to get target account ID
            let p2id_storage = match P2idNoteStorage::try_from(note.recipient().storage().items()) {
                Ok(s) => s,
                Err(_) => continue,
            };

            if p2id_storage.target() != required_recipient {
                continue;
            }

            // Check assets for the required fungible asset
            for fungible in note.assets().iter_fungible() {
                if fungible.faucet_id() == required_faucet && fungible.amount() >= required_amount {
                    payment_found = true;
                    break;
                }
            }

            if payment_found {
                break;
            }
        }
    }

    if !payment_found {
        return Err(MidenExactError::PaymentNotFound(
            "No P2ID output note found matching the required recipient, faucet, and amount. \
             Note: only NoteType::Public notes can be verified."
                .to_string(),
        ));
    }

    let payer = MidenAccountAddress::from_account_id(proven_tx.account_id()).to_string();

    Ok(v2::VerifyResponse::valid(payer))
}

/// Stub verification for when miden-native feature is not enabled.
///
/// Rejects all payments because STARK proof verification is unavailable
/// without the miden-native feature.
#[cfg(not(feature = "miden-native"))]
async fn verify_miden_payment(
    request: &types::FacilitatorVerifyRequest,
) -> Result<v2::VerifyResponse, MidenExactError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    check_requirements_match(payload, requirements)?;

    #[cfg(feature = "tracing")]
    tracing::error!(
        "miden-native feature not enabled — cannot verify STARK proofs. \
         Enable the miden-native feature for production use."
    );

    Err(MidenExactError::InvalidProof(
        "STARK proof verification unavailable: miden-native feature not enabled. \
         Cannot accept payments without cryptographic verification."
            .to_string(),
    ))
}

/// Settles a Miden payment by submitting the proven transaction.
///
/// This function:
/// 1. Re-verifies the payment
/// 2. Submits the ProvenTransaction to the Miden node
/// 3. Returns the transaction ID
async fn settle_miden_payment(
    provider: &MidenChainProvider,
    request: &types::FacilitatorSettleRequest,
) -> Result<v2::SettleResponse, MidenExactError> {
    // First verify
    verify_miden_payment(request).await?;

    let miden_payload = &request.payment_payload.payload;

    // Decode the proven transaction bytes from hex
    let proven_tx_bytes = hex::decode(&miden_payload.proven_transaction).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in proven_transaction: {e}"))
    })?;

    // Submit to the Miden node
    let tx_id = provider
        .submit_proven_transaction(&proven_tx_bytes)
        .await
        .map_err(|e| MidenExactError::ProviderError(e.to_string()))?;

    let network = provider.chain_id().to_string();

    Ok(v2::SettleResponse::Success {
        payer: miden_payload.from.to_string(),
        transaction: tx_id,
        network,
    })
}
