//! Miden chain provider for facilitator operations.
//!
//! This module provides [`MidenChainProvider`] which wraps a connection
//! to a Miden node for querying state (e.g., account balances).

use x402_types::chain::{ChainId, ChainProviderOps};

use super::{MidenChainConfig, MidenChainReference};

/// Provider for interacting with a Miden node.
///
/// This provider is used by the facilitator to:
/// - Query account state (for balance verification)
///
/// Transaction submission is handled by the agent directly in
/// bobbinth's lightweight design.
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
    /// Tracks whether the genesis commitment has already been set on the
    /// gRPC client, so we skip the RPC call on subsequent invocations.
    #[cfg(feature = "miden-client-native")]
    genesis_committed: std::sync::atomic::AtomicBool,
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
            #[cfg(feature = "miden-client-native")]
            genesis_committed: std::sync::atomic::AtomicBool::new(false),
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

    /// Ensures the gRPC client has the genesis commitment set.
    ///
    /// Uses an `AtomicBool` to skip the RPC call on subsequent invocations.
    /// The first call fetches the genesis block header and sets the commitment;
    /// all later calls return immediately.
    #[cfg(feature = "miden-client-native")]
    async fn ensure_genesis_commitment(&self) -> Result<(), MidenProviderError> {
        use std::sync::atomic::Ordering;

        // Fast path: already committed
        if self.genesis_committed.load(Ordering::Acquire) {
            return Ok(());
        }

        use miden_client::rpc::NodeRpcClient;
        use miden_protocol::block::BlockNumber;

        let (genesis_header, _) = self
            .rpc_client
            .get_block_header_by_number(Some(BlockNumber::GENESIS), false)
            .await
            .map_err(|e| {
                MidenProviderError::ConnectionError(format!(
                    "Failed to fetch genesis block header: {e}"
                ))
            })?;

        self.rpc_client
            .set_genesis_commitment(genesis_header.commitment())
            .await
            .map_err(|e| {
                MidenProviderError::ConnectionError(format!(
                    "Failed to set genesis commitment: {e}"
                ))
            })?;

        self.genesis_committed.store(true, Ordering::Release);
        Ok(())
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

            // Ensure genesis commitment is set before querying
            self.ensure_genesis_commitment().await?;

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
        // In bobbinth's lightweight design, the facilitator does not
        // sign or submit transactions. Return empty.
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

    /// Failed to query account state.
    #[error("Query error: {0}")]
    QueryError(String),
}
