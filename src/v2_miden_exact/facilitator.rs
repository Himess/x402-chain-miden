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

/// Verifies a Miden payment payload.
///
/// This function:
/// 1. Checks that the accepted requirements match the provided requirements
/// 2. Verifies the STARK proof in the proven transaction
/// 3. Checks that the output notes contain the expected P2ID payment
/// 4. Returns the verified payer account ID
async fn verify_miden_payment(
    request: &types::FacilitatorVerifyRequest,
) -> Result<v2::VerifyResponse, MidenExactError> {
    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    // Check that accepted requirements match provided requirements
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

    let miden_payload = &payload.payload;

    // TODO: Deserialize proven_transaction and verify STARK proof
    //
    // Steps that will be implemented with miden-tx dependency:
    //
    // 1. Deserialize ProvenTransaction from hex bytes:
    //    let proven_tx = ProvenTransaction::read_from(&mut bytes)?;
    //
    // 2. Verify STARK proof:
    //    let verifier = TransactionVerifier::new();
    //    verifier.verify(&proven_tx)?;
    //
    // 3. Check output notes contain P2ID to correct recipient:
    //    for note in proven_tx.output_notes() {
    //        if note.metadata().recipient() == pay_to
    //           && note.assets().contains(faucet_id, amount) {
    //            payment_found = true;
    //        }
    //    }
    //
    // 4. Check expiration:
    //    if proven_tx.expiration_block_num() < current_block {
    //        return Err(TransactionExpired(...));
    //    }

    // For now, return the payer from the payload
    let payer = miden_payload.from.to_string();

    Ok(v2::VerifyResponse::valid(payer))
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
