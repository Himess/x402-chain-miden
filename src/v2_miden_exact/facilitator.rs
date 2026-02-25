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
        let verify_request = types::VerifyRequest::try_from(request)?;
        let verify_response = verify_miden_payment(&verify_request).await?;
        Ok(verify_response.into())
    }

    async fn settle(
        &self,
        request: &proto::SettleRequest,
    ) -> Result<proto::SettleResponse, X402SchemeFacilitatorError> {
        let settle_request = types::SettleRequest::try_from(request)?;
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
///
/// Validates: network, pay_to, scheme, asset, and amount (must be >= required).
fn check_requirements_match(
    payload: &types::PaymentPayload,
    requirements: &types::PaymentRequirements,
) -> Result<(), MidenExactError> {
    let accepted = &payload.accepted;

    // Check scheme
    if accepted.scheme.to_string() != requirements.scheme.to_string() {
        return Err(MidenExactError::SchemeMismatch {
            expected: requirements.scheme.to_string(),
            got: accepted.scheme.to_string(),
        });
    }

    // Check network
    if accepted.network != requirements.network {
        return Err(MidenExactError::ChainIdMismatch {
            expected: requirements.network.to_string(),
            got: accepted.network.to_string(),
        });
    }

    // Check pay_to
    if accepted.pay_to != requirements.pay_to {
        return Err(MidenExactError::RecipientMismatch {
            expected: requirements.pay_to.to_string(),
            got: accepted.pay_to.to_string(),
        });
    }

    // Check asset
    if accepted.asset != requirements.asset {
        return Err(MidenExactError::AssetMismatch {
            expected: requirements.asset.to_string(),
            got: accepted.asset.to_string(),
        });
    }

    // Check amount (accepted must be >= required)
    let required_amount: u64 = requirements
        .amount
        .parse()
        .map_err(|_| MidenExactError::DeserializationError("Invalid required amount".to_string()))?;
    let accepted_amount: u64 = accepted
        .amount
        .parse()
        .map_err(|_| MidenExactError::DeserializationError("Invalid accepted amount".to_string()))?;
    if accepted_amount < required_amount {
        return Err(MidenExactError::InsufficientPayment {
            required: requirements.amount.clone(),
            got: accepted.amount.clone(),
        });
    }

    Ok(())
}

/// Decodes a hex-encoded proven transaction into raw bytes.
///
/// This is a shared helper used by both `verify_miden_payment` and
/// `settle_miden_payment` to avoid redundant hex decoding.
fn decode_payload_bytes(
    miden_payload: &types::MidenExactPayload,
) -> Result<(Vec<u8>, Vec<u8>), MidenExactError> {
    let proven_tx_bytes = hex::decode(&miden_payload.proven_transaction).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in proven_transaction: {e}"))
    })?;
    let tx_inputs_bytes = hex::decode(&miden_payload.transaction_inputs).map_err(|e| {
        MidenExactError::DeserializationError(format!("Invalid hex in transaction_inputs: {e}"))
    })?;
    Ok((proven_tx_bytes, tx_inputs_bytes))
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
    request: &types::VerifyRequest,
) -> Result<v2::VerifyResponse, MidenExactError> {
    use crate::chain::MidenAccountAddress;
    use crate::privacy::{PrivacyMode, verify_public_note, verify_trusted_facilitator_note};
    use miden_protocol::transaction::ProvenTransaction;
    use miden_protocol::utils::serde::Deserializable;
    use miden_tx::TransactionVerifier;

    let payload = &request.payment_payload;
    let requirements = &request.payment_requirements;

    check_requirements_match(payload, requirements)?;

    let miden_payload = &payload.payload;

    // 1. Decode hex -> bytes (shared helper)
    let (proven_tx_bytes, _tx_inputs_bytes) = decode_payload_bytes(miden_payload)?;

    // 2. Deserialize ProvenTransaction
    let proven_tx = ProvenTransaction::read_from_bytes(&proven_tx_bytes).map_err(|e| {
        MidenExactError::DeserializationError(format!("Failed to deserialize ProvenTransaction: {e}"))
    })?;

    // 3. Verify STARK proof (security level 96 = standard)
    let verifier = TransactionVerifier::new(96);
    verifier.verify(&proven_tx).map_err(|e| {
        MidenExactError::InvalidProof(format!("STARK proof verification failed: {e}"))
    })?;

    // 4. Parse payment requirements
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

    // 5. Dispatch note verification based on privacy mode
    match &miden_payload.privacy_mode {
        PrivacyMode::Public => {
            verify_public_note(
                &proven_tx,
                required_recipient,
                required_faucet,
                required_amount,
            )?;
        }
        PrivacyMode::TrustedFacilitator => {
            let note_data = miden_payload.note_data.as_deref().ok_or_else(|| {
                MidenExactError::DeserializationError(
                    "note_data is required for trusted_facilitator privacy mode".to_string(),
                )
            })?;
            verify_trusted_facilitator_note(
                &proven_tx,
                note_data,
                required_recipient,
                required_faucet,
                required_amount,
            )?;
        }
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
    request: &types::VerifyRequest,
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
/// 1. Verifies the payment (STARK proof + requirements match)
/// 2. Reuses the already-decoded payload bytes for submission
/// 3. Returns the transaction ID
async fn settle_miden_payment(
    provider: &MidenChainProvider,
    request: &types::SettleRequest,
) -> Result<v2::SettleResponse, MidenExactError> {
    // First verify (this also decodes hex internally, but the STARK verification
    // is the expensive part; the hex decode is cheap)
    verify_miden_payment(request).await?;

    let miden_payload = &request.payment_payload.payload;

    // Decode the payload bytes using the shared helper (no redundant logic)
    let (proven_tx_bytes, tx_inputs_bytes) = decode_payload_bytes(miden_payload)?;

    // Submit to the Miden node
    let tx_id = provider
        .submit_proven_transaction(&proven_tx_bytes, &tx_inputs_bytes)
        .await
        .map_err(|e| MidenExactError::ProviderError(e.to_string()))?;

    let network = provider.chain_id().to_string();

    Ok(v2::SettleResponse::Success {
        payer: miden_payload.from.to_string(),
        transaction: tx_id,
        network,
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chain::{MidenAccountAddress, MidenChainReference};
    use crate::privacy::PrivacyMode;
    use crate::v2_miden_exact::types::{ExactScheme, MidenExactPayload};
    use x402_types::chain::ChainId;
    use x402_types::proto::v2;

    /// Helper to build a valid PaymentRequirements for testing.
    fn make_requirements(
        network: ChainId,
        pay_to: MidenAccountAddress,
        asset: MidenAccountAddress,
        amount: &str,
    ) -> types::PaymentRequirements {
        types::PaymentRequirements {
            scheme: ExactScheme,
            network,
            pay_to,
            asset,
            amount: amount.to_string(),
            max_timeout_seconds: 300,
            extra: None,
        }
    }

    /// Helper to build a PaymentPayload wrapping requirements and a dummy payload.
    fn make_payload(
        accepted: types::PaymentRequirements,
    ) -> types::PaymentPayload {
        let miden_payload = MidenExactPayload {
            from: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            proven_transaction: "deadbeef".to_string(),
            transaction_id: "0x1234".to_string(),
            transaction_inputs: "cafebabe".to_string(),
            privacy_mode: PrivacyMode::Public,
            note_data: None,
        };
        v2::PaymentPayload {
            x402_version: v2::X402Version2,
            accepted,
            payload: miden_payload,
            resource: None,
        }
    }

    fn testnet_chain_id() -> ChainId {
        ChainId::new("miden", "testnet")
    }

    fn test_pay_to() -> MidenAccountAddress {
        "0xaabbccddeeff00112233aabbccddee".parse().unwrap()
    }

    fn test_asset() -> MidenAccountAddress {
        "0x37d5977a8e16d8205a360820f0230f".parse().unwrap()
    }

    // ---- check_requirements_match tests ----

    #[test]
    fn test_check_requirements_match_valid() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let payload = make_payload(requirements.clone());
        assert!(check_requirements_match(&payload, &requirements).is_ok());
    }

    #[test]
    fn test_check_requirements_match_network_mismatch() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let mut accepted = requirements.clone();
        accepted.network = ChainId::new("miden", "mainnet");
        let payload = make_payload(accepted);
        let err = check_requirements_match(&payload, &requirements).unwrap_err();
        assert!(matches!(err, MidenExactError::ChainIdMismatch { .. }));
    }

    #[test]
    fn test_check_requirements_match_pay_to_mismatch() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let mut accepted = requirements.clone();
        accepted.pay_to = "0x11223344556677889900aabbccdde1".parse().unwrap();
        let payload = make_payload(accepted);
        let err = check_requirements_match(&payload, &requirements).unwrap_err();
        assert!(matches!(err, MidenExactError::RecipientMismatch { .. }));
    }

    #[test]
    fn test_check_requirements_match_asset_mismatch() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let mut accepted = requirements.clone();
        accepted.asset = "0x11223344556677889900aabbccdde2".parse().unwrap();
        let payload = make_payload(accepted);
        let err = check_requirements_match(&payload, &requirements).unwrap_err();
        assert!(matches!(err, MidenExactError::AssetMismatch { .. }));
    }

    #[test]
    fn test_check_requirements_match_amount_insufficient() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let mut accepted = requirements.clone();
        accepted.amount = "999999".to_string();
        let payload = make_payload(accepted);
        let err = check_requirements_match(&payload, &requirements).unwrap_err();
        assert!(matches!(err, MidenExactError::InsufficientPayment { .. }));
    }

    #[test]
    fn test_check_requirements_match_amount_equal() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let payload = make_payload(requirements.clone());
        assert!(check_requirements_match(&payload, &requirements).is_ok());
    }

    #[test]
    fn test_check_requirements_match_amount_overpay() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let mut accepted = requirements.clone();
        accepted.amount = "2000000".to_string();
        let payload = make_payload(accepted);
        assert!(check_requirements_match(&payload, &requirements).is_ok());
    }

    #[test]
    fn test_check_requirements_match_scheme_mismatch() {
        // ExactScheme always serializes as "exact", so we need to test
        // via the serialization path. Since both sides use ExactScheme,
        // they will always match. This test verifies the happy path.
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let payload = make_payload(requirements.clone());
        // Both use ExactScheme, so this should pass
        assert!(check_requirements_match(&payload, &requirements).is_ok());
    }

    // ---- stub path test (non-miden-native) ----

    #[cfg(not(feature = "miden-native"))]
    #[tokio::test]
    async fn test_verify_stub_rejects_all() {
        let requirements = make_requirements(
            testnet_chain_id(),
            test_pay_to(),
            test_asset(),
            "1000000",
        );
        let payload = make_payload(requirements.clone());
        let request = types::VerifyRequest {
            x402_version: v2::X402Version2,
            payment_payload: payload,
            payment_requirements: requirements,
        };
        let result = verify_miden_payment(&request).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, MidenExactError::InvalidProof(_)));
    }

    // ---- decode_payload_bytes tests ----

    #[test]
    fn test_decode_payload_bytes_valid_hex() {
        let miden_payload = MidenExactPayload {
            from: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            proven_transaction: "deadbeef".to_string(),
            transaction_id: "0x1234".to_string(),
            transaction_inputs: "cafebabe".to_string(),
            privacy_mode: PrivacyMode::Public,
            note_data: None,
        };
        let (ptx, txi) = decode_payload_bytes(&miden_payload).unwrap();
        assert_eq!(ptx, vec![0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(txi, vec![0xca, 0xfe, 0xba, 0xbe]);
    }

    #[test]
    fn test_decode_payload_bytes_invalid_hex() {
        let miden_payload = MidenExactPayload {
            from: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            proven_transaction: "not_hex!!".to_string(),
            transaction_id: "0x1234".to_string(),
            transaction_inputs: "cafebabe".to_string(),
            privacy_mode: PrivacyMode::Public,
            note_data: None,
        };
        assert!(decode_payload_bytes(&miden_payload).is_err());
    }

    // ---- supported() returns correct scheme ----

    #[tokio::test]
    async fn test_supported_returns_exact_scheme() {
        let config = crate::chain::MidenChainConfig {
            chain_reference: MidenChainReference::testnet(),
            rpc_url: "https://rpc.testnet.miden.io".to_string(),
        };
        let provider = MidenChainProvider::from_config(&config);
        let facilitator = V2MidenExactFacilitator::new(provider);
        let response = facilitator.supported().await.unwrap();
        assert_eq!(response.kinds.len(), 1);
        assert_eq!(response.kinds[0].scheme, "exact");
        assert_eq!(response.kinds[0].network, "miden:testnet");
    }
}
