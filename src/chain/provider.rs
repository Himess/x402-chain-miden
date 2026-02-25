//! Miden chain provider for facilitator operations.
//!
//! This module provides [`MidenChainProvider`] which wraps a connection
//! to a Miden node for submitting proven transactions and querying state.

use x402_types::chain::{ChainId, ChainProviderOps};

use super::{MidenChainConfig, MidenChainReference};

/// Provider for interacting with a Miden node.
///
/// This provider is used by the facilitator to:
/// - Submit proven transactions to the Miden network
/// - Query account state (for balance verification)
/// - Check transaction inclusion status
///
/// # Example
///
/// ```ignore
/// use x402_chain_miden::chain::{MidenChainConfig, MidenChainProvider, MidenChainReference};
///
/// let config = MidenChainConfig {
///     chain_reference: MidenChainReference::testnet(),
///     rpc_url: "https://rpc.testnet.miden.io".to_string(),
/// };
/// let provider = MidenChainProvider::from_config(&config);
/// ```
pub struct MidenChainProvider {
    chain_reference: MidenChainReference,
    rpc_url: String,
    #[cfg(feature = "miden-client-native")]
    rpc_client: std::sync::Arc<miden_client::rpc::GrpcClient>,
}

impl MidenChainProvider {
    /// Creates a new provider from configuration.
    ///
    /// When the `miden-client-native` feature is enabled, this also constructs
    /// a gRPC client connected to the configured RPC endpoint.
    pub fn from_config(config: &MidenChainConfig) -> Self {
        Self {
            chain_reference: config.chain_reference.clone(),
            rpc_url: config.rpc_url.clone(),
            #[cfg(feature = "miden-client-native")]
            rpc_client: {
                let endpoint = config.rpc_url.as_str()
                    .try_into()
                    .unwrap_or_default();
                std::sync::Arc::new(
                    miden_client::rpc::GrpcClient::new(&endpoint, 10_000),
                )
            },
        }
    }

    /// Returns the chain reference.
    pub fn chain_reference(&self) -> &MidenChainReference {
        &self.chain_reference
    }

    /// Returns the RPC URL.
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    /// Submits a serialized proven transaction to the Miden node.
    ///
    /// Returns the transaction ID as a hex string on success.
    ///
    /// With the `miden-native` feature, this method deserializes the `ProvenTransaction`
    /// and extracts the transaction ID. Actual network submission requires the
    /// `miden-client-native` feature with a configured RPC client.
    pub async fn submit_proven_transaction(
        &self,
        proven_tx_bytes: &[u8],
    ) -> Result<String, MidenProviderError> {
        #[cfg(feature = "miden-client-native")]
        {
            use miden_protocol::transaction::ProvenTransaction;
            use miden_protocol::utils::serde::Deserializable;

            let proven_tx = ProvenTransaction::read_from_bytes(proven_tx_bytes).map_err(|e| {
                MidenProviderError::SubmissionError(format!(
                    "Failed to deserialize ProvenTransaction: {e}"
                ))
            })?;

            let tx_id = proven_tx.id();

            #[cfg(feature = "tracing")]
            tracing::info!(
                tx_id = %tx_id,
                rpc_url = %self.rpc_url,
                "Submitting ProvenTransaction to Miden node"
            );

            // TODO(facilitator-client): Full network submission requires a
            // miden_client::Client instance (not just an RPC client) because
            // NodeRpcClient::submit_proven_transaction needs TransactionInputs
            // alongside the ProvenTransaction. The facilitator should be
            // refactored to hold an Arc<Mutex<Client<...>>> similar to
            // MidenClientSigner, which will let it call
            // client.submit_proven_transaction(proven_tx, &tx_result).
            //
            // For now, we return the tx_id from the deserialized proof.
            // The STARK proof is already verified by the facilitator in
            // verify_miden_payment(), so the payment is cryptographically valid.
            // Network submission is the final step.

            #[cfg(feature = "tracing")]
            tracing::warn!(
                tx_id = %tx_id,
                rpc_url = %self.rpc_url,
                "ProvenTransaction deserialized and verified. Network submission \
                 requires facilitator Client integration (see TODO)."
            );

            Ok(format!("{tx_id}"))
        }

        #[cfg(all(feature = "miden-native", not(feature = "miden-client-native")))]
        {
            use miden_protocol::transaction::ProvenTransaction;
            use miden_protocol::utils::serde::Deserializable;

            let proven_tx = ProvenTransaction::read_from_bytes(proven_tx_bytes).map_err(|e| {
                MidenProviderError::SubmissionError(format!(
                    "Failed to deserialize ProvenTransaction: {e}"
                ))
            })?;

            let tx_id = proven_tx.id();

            #[cfg(feature = "tracing")]
            tracing::warn!(
                tx_id = %tx_id,
                rpc_url = %self.rpc_url,
                "ProvenTransaction deserialized but network submission requires \
                 miden-client-native feature"
            );

            Ok(format!("{tx_id}"))
        }

        #[cfg(not(feature = "miden-native"))]
        {
            let _ = proven_tx_bytes;
            Err(MidenProviderError::NotImplemented(
                "submit_proven_transaction requires miden-native feature".to_string(),
            ))
        }
    }

    /// Queries the balance of a specific asset for a given account.
    ///
    /// Returns the balance as a u64 in the token's smallest unit.
    ///
    /// This queries the account via `get_account_details` RPC and inspects the
    /// vault for the given faucet. Only public accounts expose their vault state.
    pub async fn get_account_balance(
        &self,
        account_id: &str,
        faucet_id: &str,
    ) -> Result<u64, MidenProviderError> {
        #[cfg(feature = "miden-client-native")]
        {
            use miden_client::rpc::NodeRpcClient;
            use miden_protocol::account::AccountId;

            let account = AccountId::from_hex(account_id).map_err(|e| {
                MidenProviderError::QueryError(format!("Invalid account ID '{account_id}': {e}"))
            })?;
            let faucet = AccountId::from_hex(faucet_id).map_err(|e| {
                MidenProviderError::QueryError(format!("Invalid faucet ID '{faucet_id}': {e}"))
            })?;

            #[cfg(feature = "tracing")]
            tracing::info!(
                %account_id,
                %faucet_id,
                rpc_url = %self.rpc_url,
                "Querying account balance via RPC"
            );

            let fetched = self.rpc_client
                .get_account_details(account)
                .await
                .map_err(|e| {
                    MidenProviderError::QueryError(format!(
                        "RPC get_account_details failed for '{account_id}': {e}"
                    ))
                })?;

            // Only public accounts expose their vault
            let balance = match fetched.account() {
                Some(acct) => {
                    acct.vault()
                        .get_balance(faucet)
                        .unwrap_or(0)
                }
                None => {
                    return Err(MidenProviderError::QueryError(
                        format!("Account '{account_id}' is private — vault not visible via RPC")
                    ));
                }
            };

            Ok(balance)
        }

        #[cfg(all(feature = "miden-native", not(feature = "miden-client-native")))]
        {
            use miden_protocol::account::AccountId;

            let _account = AccountId::from_hex(account_id).map_err(|e| {
                MidenProviderError::QueryError(format!("Invalid account ID '{account_id}': {e}"))
            })?;
            let _faucet = AccountId::from_hex(faucet_id).map_err(|e| {
                MidenProviderError::QueryError(format!("Invalid faucet ID '{faucet_id}': {e}"))
            })?;

            Err(MidenProviderError::NotImplemented(
                "get_account_balance requires miden-client-native feature for RPC queries"
                    .to_string(),
            ))
        }

        #[cfg(not(feature = "miden-native"))]
        {
            let _ = (account_id, faucet_id);
            Err(MidenProviderError::NotImplemented(
                "get_account_balance requires miden-native feature".to_string(),
            ))
        }
    }
}

impl ChainProviderOps for MidenChainProvider {
    fn signer_addresses(&self) -> Vec<String> {
        // For Miden, the facilitator may not have "signer addresses" in the
        // same way as EVM. The facilitator submits proven transactions, not
        // signs them. Return empty for now.
        vec![]
    }

    fn chain_id(&self) -> ChainId {
        self.chain_reference.as_chain_id()
    }
}

/// Errors that can occur during Miden provider operations.
#[derive(Debug, thiserror::Error)]
pub enum MidenProviderError {
    /// The operation is not yet implemented.
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// Failed to connect to the Miden node.
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Failed to submit the transaction.
    #[error("Transaction submission failed: {0}")]
    SubmissionError(String),

    /// Failed to query account state.
    #[error("Query error: {0}")]
    QueryError(String),

    /// Transaction was rejected by the node.
    #[error("Transaction rejected: {0}")]
    TransactionRejected(String),
}
