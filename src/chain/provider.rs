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
}

impl MidenChainProvider {
    /// Creates a new provider from configuration.
    pub fn from_config(config: &MidenChainConfig) -> Self {
        Self {
            chain_reference: config.chain_reference.clone(),
            rpc_url: config.rpc_url.clone(),
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
    pub async fn submit_proven_transaction(
        &self,
        _proven_tx_bytes: &[u8],
    ) -> Result<String, MidenProviderError> {
        // TODO: Implement actual RPC call to Miden node
        // This will use the miden-client's RPC client to submit
        // the ProvenTransaction to the network.
        //
        // Pseudocode:
        //   let client = MidenClient::connect(&self.rpc_url).await?;
        //   let proven_tx = ProvenTransaction::read_from(&mut proven_tx_bytes)?;
        //   let tx_id = client.submit_transaction(proven_tx).await?;
        //   Ok(tx_id.to_hex())
        Err(MidenProviderError::NotImplemented(
            "submit_proven_transaction requires miden-client integration".to_string(),
        ))
    }

    /// Queries the balance of a specific asset for a given account.
    ///
    /// Returns the balance as a u64 in the token's smallest unit.
    pub async fn get_account_balance(
        &self,
        _account_id: &str,
        _faucet_id: &str,
    ) -> Result<u64, MidenProviderError> {
        // TODO: Implement actual RPC call to query account vault
        //
        // Pseudocode:
        //   let client = MidenClient::connect(&self.rpc_url).await?;
        //   let account = client.get_account(account_id).await?;
        //   let balance = account.vault().get_balance(faucet_id)?;
        //   Ok(balance)
        Err(MidenProviderError::NotImplemented(
            "get_account_balance requires miden-client integration".to_string(),
        ))
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
