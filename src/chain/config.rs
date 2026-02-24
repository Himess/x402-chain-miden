//! Configuration types for connecting to a Miden node.
//!
//! This module provides configuration structures used to initialize
//! a Miden chain provider for facilitator operations.

use serde::{Deserialize, Serialize};

use super::MidenChainReference;

/// Configuration for a Miden chain connection.
///
/// This configuration is used to initialize a [`MidenChainProvider`](super::provider::MidenChainProvider)
/// for facilitator-side operations (verification and settlement).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MidenChainConfig {
    /// The chain reference (e.g., `testnet`, `mainnet`).
    pub chain_reference: MidenChainReference,
    /// The Miden node RPC endpoint URL.
    pub rpc_url: String,
}
