//! Core Miden chain types, configuration, and provider.
//!
//! This module provides the fundamental types for interacting with
//! the Miden blockchain within the x402 protocol:
//!
//! - [`MidenAccountAddress`] - Miden account ID wrapper with serialization
//! - [`MidenChainReference`] - Chain reference (`testnet` or `mainnet`)
//! - [`MidenTokenDeployment`] - Token (faucet) deployment info
//! - [`MidenChainConfig`] - Configuration for connecting to a Miden node

pub mod types;
pub use types::*;

pub mod config;
pub use config::*;

#[cfg(feature = "facilitator")]
pub mod provider;
#[cfg(feature = "facilitator")]
pub use provider::*;
