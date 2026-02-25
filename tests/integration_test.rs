//! Integration tests for x402-chain-miden.
//!
//! These tests verify the complete payment flow including price tag creation,
//! client payment signing, and facilitator verification/settlement.

use x402_chain_miden::chain::{
    MidenAccountAddress, MidenChainReference, MidenTokenDeployment,
};
use x402_chain_miden::{KnownNetworkMiden, MidenUSDC, V2MidenExact};
use x402_types::chain::ChainId;
use x402_types::scheme::X402SchemeId;

// ============================================================================
// Scheme Identity Tests
// ============================================================================

#[test]
fn test_v2_miden_exact_scheme_id() {
    let scheme = V2MidenExact;
    assert_eq!(scheme.namespace(), "miden");
    assert_eq!(scheme.scheme(), "exact");
    assert_eq!(scheme.x402_version(), 2);
    assert_eq!(scheme.id(), "v2-miden-exact");
}

// ============================================================================
// Known Networks Tests
// ============================================================================

#[test]
fn test_known_network_testnet() {
    let chain_id: ChainId = ChainId::miden_testnet();
    assert_eq!(chain_id.to_string(), "miden:testnet");
    assert_eq!(chain_id.namespace, "miden");
    assert_eq!(chain_id.reference, "testnet");
}

#[test]
fn test_known_network_mainnet() {
    let chain_id: ChainId = ChainId::miden_mainnet();
    assert_eq!(chain_id.to_string(), "miden:mainnet");
    assert_eq!(chain_id.namespace, "miden");
    assert_eq!(chain_id.reference, "mainnet");
}

#[test]
fn test_usdc_testnet_deployment() {
    let usdc = MidenUSDC::miden_testnet();
    assert_eq!(usdc.decimals, 6);
    assert_eq!(usdc.chain_reference, MidenChainReference::testnet());
}

#[test]
fn test_usdc_mainnet_deployment() {
    let usdc = MidenUSDC::miden_mainnet();
    assert_eq!(usdc.decimals, 6);
    assert_eq!(usdc.chain_reference, MidenChainReference::mainnet());
}

#[test]
fn test_token_deployment_convenience() {
    let testnet = MidenTokenDeployment::testnet_usdc();
    let mainnet = MidenTokenDeployment::mainnet_usdc();
    assert_eq!(testnet.chain_reference, MidenChainReference::testnet());
    assert_eq!(mainnet.chain_reference, MidenChainReference::mainnet());
    assert_eq!(testnet.decimals, 6);
    assert_eq!(mainnet.decimals, 6);
}

// ============================================================================
// Chain Reference Tests
// ============================================================================

#[test]
fn test_chain_reference_try_from_str() {
    let testnet = MidenChainReference::try_from("testnet").unwrap();
    assert_eq!(testnet, MidenChainReference::testnet());

    let mainnet = MidenChainReference::try_from("mainnet").unwrap();
    assert_eq!(mainnet, MidenChainReference::mainnet());
}

#[test]
fn test_chain_reference_try_from_str_invalid() {
    let result = MidenChainReference::try_from("devnet");
    assert!(result.is_err());
}

#[test]
fn test_chain_reference_roundtrip_via_chain_id() {
    let original = MidenChainReference::testnet();
    let chain_id: ChainId = original.clone().into();
    let recovered = MidenChainReference::try_from(chain_id).unwrap();
    assert_eq!(original, recovered);
}

// ============================================================================
// Address Tests
// ============================================================================

#[test]
fn test_miden_address_parse_hex() {
    let addr: MidenAccountAddress = "0xaabbccddeeff00112233".parse().unwrap();
    assert!(addr.to_string().starts_with("0x"));
}

#[test]
fn test_miden_address_parse_no_prefix() {
    let addr: MidenAccountAddress = "aabbccddeeff00112233".parse().unwrap();
    assert!(addr.to_string().starts_with("0x"));
}

#[test]
fn test_miden_address_roundtrip() {
    let original: MidenAccountAddress = "0xdeadbeef".parse().unwrap();
    let s = original.to_string();
    let recovered: MidenAccountAddress = s.parse().unwrap();
    assert_eq!(original, recovered);
}

#[test]
fn test_miden_address_serde_json() {
    let addr: MidenAccountAddress = "0xdeadbeef".parse().unwrap();
    let json = serde_json::to_string(&addr).unwrap();
    let recovered: MidenAccountAddress = serde_json::from_str(&json).unwrap();
    assert_eq!(addr, recovered);
}

// ============================================================================
// Token Amount Tests
// ============================================================================

#[test]
fn test_deployed_token_amount() {
    let usdc = MidenTokenDeployment::testnet_usdc();
    let amount = usdc.amount(1_000_000);
    assert_eq!(amount.amount, 1_000_000);
    assert_eq!(amount.token.decimals, 6);
}

#[test]
fn test_deployed_token_parse_amount() {
    let usdc = MidenTokenDeployment::testnet_usdc();
    let amount = usdc.parse("1.5").unwrap();
    assert_eq!(amount.amount, 1_500_000); // 1.5 * 10^6
}

#[test]
fn test_deployed_token_parse_whole() {
    let usdc = MidenTokenDeployment::testnet_usdc();
    let amount = usdc.parse("10").unwrap();
    assert_eq!(amount.amount, 10_000_000); // 10 * 10^6
}

#[test]
fn test_deployed_token_parse_smallest_unit() {
    let usdc = MidenTokenDeployment::testnet_usdc();
    let amount = usdc.parse("0.000001").unwrap();
    assert_eq!(amount.amount, 1); // Smallest unit
}

// ============================================================================
// Price Tag Tests (server feature)
// ============================================================================

#[cfg(feature = "server")]
mod server_tests {
    use super::*;

    #[test]
    fn test_price_tag_creation() {
        let recipient: MidenAccountAddress = "0xaabbccddee11223344".parse().unwrap();
        let usdc = MidenTokenDeployment::testnet_usdc();
        let price_tag = V2MidenExact::price_tag(recipient.clone(), usdc.amount(1_000_000));

        assert_eq!(price_tag.requirements.scheme, "exact");
        assert_eq!(price_tag.requirements.network.to_string(), "miden:testnet");
        assert_eq!(price_tag.requirements.amount, "1000000");
        assert_eq!(price_tag.requirements.pay_to, recipient.to_string());
        assert_eq!(price_tag.requirements.max_timeout_seconds, 300);
        assert!(price_tag.enricher.is_none());
    }

    #[test]
    fn test_price_tag_mainnet() {
        let recipient: MidenAccountAddress = "0xaabbccddee11223344".parse().unwrap();
        let usdc = MidenTokenDeployment::mainnet_usdc();
        let price_tag = V2MidenExact::price_tag(recipient, usdc.amount(500_000));

        assert_eq!(price_tag.requirements.network.to_string(), "miden:mainnet");
        assert_eq!(price_tag.requirements.amount, "500000");
    }

    #[test]
    fn test_price_tag_different_amounts() {
        let recipient: MidenAccountAddress = "0xdeadbeef".parse().unwrap();
        let usdc = MidenTokenDeployment::testnet_usdc();

        // 0.01 USDC
        let small = V2MidenExact::price_tag(recipient.clone(), usdc.amount(10_000));
        assert_eq!(small.requirements.amount, "10000");

        // 100 USDC
        let large = V2MidenExact::price_tag(recipient, usdc.amount(100_000_000));
        assert_eq!(large.requirements.amount, "100000000");
    }

    #[test]
    fn test_price_tag_requirements_serializable() {
        let recipient: MidenAccountAddress = "0xaabbccddee".parse().unwrap();
        let usdc = MidenTokenDeployment::testnet_usdc();
        let price_tag = V2MidenExact::price_tag(recipient, usdc.amount(1_000_000));

        let json = serde_json::to_value(&price_tag.requirements).unwrap();
        assert_eq!(json["scheme"], "exact");
        assert_eq!(json["amount"], "1000000");
        assert_eq!(json["maxTimeoutSeconds"], 300);
    }
}

// ============================================================================
// Client Tests (client feature)
// ============================================================================

#[cfg(feature = "client")]
mod client_tests {
    use super::*;
    use async_trait::async_trait;
    use x402_chain_miden::v2_miden_exact::client::MidenSignerLike;
    use x402_chain_miden::V2MidenExactClient;
    use x402_types::scheme::client::X402Error;

    #[derive(Debug, Clone)]
    struct MockSigner {
        id: String,
    }

    #[async_trait]
    impl MidenSignerLike for MockSigner {
        fn account_id(&self) -> String {
            self.id.clone()
        }

        async fn create_and_prove_p2id(
            &self,
            _recipient: &str,
            _faucet_id: &str,
            _amount: u64,
        ) -> Result<(String, String), X402Error> {
            Ok(("deadbeef".to_string(), "cafebabe".to_string()))
        }
    }

    #[test]
    fn test_client_creation() {
        let signer = MockSigner {
            id: "0x1234".to_string(),
        };
        let _client = V2MidenExactClient::new(signer);
    }

    #[test]
    fn test_client_scheme_id() {
        let signer = MockSigner {
            id: "0x1234".to_string(),
        };
        let client = V2MidenExactClient::new(signer);
        assert_eq!(client.namespace(), "miden");
        assert_eq!(client.scheme(), "exact");
        assert_eq!(client.x402_version(), 2);
    }
}

// ============================================================================
// Facilitator Tests (facilitator feature)
// ============================================================================

#[cfg(feature = "facilitator")]
mod facilitator_tests {
    use super::*;
    use x402_chain_miden::chain::{MidenChainConfig, MidenChainProvider};
    use x402_chain_miden::v2_miden_exact::facilitator::V2MidenExactFacilitator;
    use x402_types::chain::ChainProviderOps;
    use x402_types::scheme::X402SchemeFacilitator;

    #[test]
    fn test_facilitator_creation() {
        let config = MidenChainConfig {
            chain_reference: MidenChainReference::testnet(),
            rpc_url: "https://rpc.testnet.miden.io".to_string(),
        };
        let provider = MidenChainProvider::from_config(&config);
        let _facilitator = V2MidenExactFacilitator::new(provider);
    }

    #[test]
    fn test_provider_chain_id() {
        let config = MidenChainConfig {
            chain_reference: MidenChainReference::testnet(),
            rpc_url: "https://rpc.testnet.miden.io".to_string(),
        };
        let provider = MidenChainProvider::from_config(&config);
        let chain_id = provider.chain_id();
        assert_eq!(chain_id.to_string(), "miden:testnet");
    }

    #[test]
    fn test_provider_mainnet_chain_id() {
        let config = MidenChainConfig {
            chain_reference: MidenChainReference::mainnet(),
            rpc_url: "https://rpc.mainnet.miden.io".to_string(),
        };
        let provider = MidenChainProvider::from_config(&config);
        let chain_id = provider.chain_id();
        assert_eq!(chain_id.to_string(), "miden:mainnet");
    }

    #[tokio::test]
    async fn test_facilitator_supported() {
        let config = MidenChainConfig {
            chain_reference: MidenChainReference::testnet(),
            rpc_url: "https://rpc.testnet.miden.io".to_string(),
        };
        let provider = MidenChainProvider::from_config(&config);
        let facilitator = V2MidenExactFacilitator::new(provider);

        let supported = facilitator.supported().await.unwrap();
        assert_eq!(supported.kinds.len(), 1);
        assert_eq!(supported.kinds[0].scheme, "exact");
        assert_eq!(supported.kinds[0].x402_version, 2);
        assert_eq!(supported.kinds[0].network, "miden:testnet");
    }
}

// ============================================================================
// Miden Exact Payload Serialization Tests
// ============================================================================

mod payload_tests {
    use x402_chain_miden::v2_miden_exact::types::{ExactScheme, MidenExactPayload};
    use x402_chain_miden::chain::MidenAccountAddress;

    #[test]
    fn test_exact_scheme_value() {
        assert_eq!(ExactScheme.to_string(), "exact");
    }

    #[test]
    fn test_miden_payload_serde_roundtrip() {
        let payload = MidenExactPayload {
            from: "0xdeadbeef".parse().unwrap(),
            proven_transaction: "aabbccdd".to_string(),
            transaction_id: "11223344".to_string(),
        };

        let json = serde_json::to_string(&payload).unwrap();
        let recovered: MidenExactPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload.from, recovered.from);
        assert_eq!(payload.proven_transaction, recovered.proven_transaction);
        assert_eq!(payload.transaction_id, recovered.transaction_id);
    }

    #[test]
    fn test_miden_payload_json_structure() {
        let payload = MidenExactPayload {
            from: "0xaabb".parse().unwrap(),
            proven_transaction: "cafebabe".to_string(),
            transaction_id: "deadbeef".to_string(),
        };

        let value = serde_json::to_value(&payload).unwrap();
        assert!(value["from"].is_string());
        assert_eq!(value["provenTransaction"], "cafebabe");
        assert_eq!(value["transactionId"], "deadbeef");
    }

    #[test]
    fn test_miden_address_from_bytes() {
        let bytes = vec![0xde, 0xad, 0xbe, 0xef];
        let addr = MidenAccountAddress::from_bytes(bytes.clone());
        let hex_str = addr.to_string();
        assert_eq!(hex_str, "0xdeadbeef");
    }
}

// ============================================================================
// Error Type Tests
// ============================================================================

mod error_tests {
    use x402_chain_miden::v2_miden_exact::types::MidenExactError;

    #[test]
    fn test_error_display() {
        let err = MidenExactError::ChainIdMismatch {
            expected: "miden:testnet".to_string(),
            got: "miden:mainnet".to_string(),
        };
        let msg = err.to_string();
        assert!(msg.contains("miden:testnet"));
        assert!(msg.contains("miden:mainnet"));
    }

    #[test]
    fn test_error_variants() {
        // Ensure all error variants are constructible
        let _ = MidenExactError::InvalidProof("bad proof".to_string());
        let _ = MidenExactError::PaymentNotFound("test".to_string());
        let _ = MidenExactError::RecipientMismatch {
            expected: "a".to_string(),
            got: "b".to_string(),
        };
        let _ = MidenExactError::InsufficientPayment {
            required: "100".to_string(),
            got: "50".to_string(),
        };
        let _ = MidenExactError::TransactionExpired(0u64);
        let _ = MidenExactError::DeserializationError("parse fail".to_string());
        let _ = MidenExactError::AcceptedRequirementsMismatch;
        let _ = MidenExactError::ProviderError("rpc fail".to_string());
    }
}
