//! Known Miden networks and token deployments.
//!
//! This module provides convenient methods to get token deployment information
//! for well-known Miden networks.

use x402_types::chain::ChainId;

use crate::chain::{MidenAccountAddress, MidenChainReference, MidenTokenDeployment};

/// Trait providing convenient methods for well-known Miden networks.
///
/// This trait can be implemented for any type to provide static methods that create
/// instances for well-known Miden blockchain networks.
///
/// # Example
///
/// ```ignore
/// use x402_chain_miden::KnownNetworkMiden;
/// use x402_types::chain::ChainId;
///
/// let testnet: ChainId = ChainId::miden_testnet();
/// assert_eq!(testnet.to_string(), "miden:testnet");
/// ```
pub trait KnownNetworkMiden<A> {
    /// Returns the instance for Miden testnet (miden:testnet).
    fn miden_testnet() -> A;
    /// Returns the instance for Miden mainnet (miden:mainnet).
    fn miden_mainnet() -> A;
}

impl KnownNetworkMiden<ChainId> for ChainId {
    fn miden_testnet() -> ChainId {
        ChainId::new("miden", "testnet")
    }

    fn miden_mainnet() -> ChainId {
        ChainId::new("miden", "mainnet")
    }
}

/// Marker type for USDC-equivalent token on Miden.
///
/// This follows the same pattern as `x402_types::networks::USDC` for EVM chains.
/// On Miden, USDC is represented as a fungible asset issued by a faucet account.
pub struct MidenUSDC;

impl KnownNetworkMiden<MidenTokenDeployment> for MidenUSDC {
    fn miden_testnet() -> MidenTokenDeployment {
        MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            // Placeholder faucet ID - will be updated when Miden testnet
            // deploys a standard USDC-equivalent faucet.
            faucet_id: MidenAccountAddress::from_bytes(vec![0; 15]),
            decimals: 6,
        }
    }

    fn miden_mainnet() -> MidenTokenDeployment {
        MidenTokenDeployment {
            chain_reference: MidenChainReference::mainnet(),
            // Placeholder faucet ID - will be updated at mainnet launch.
            faucet_id: MidenAccountAddress::from_bytes(vec![0; 15]),
            decimals: 6,
        }
    }
}

impl MidenTokenDeployment {
    /// Returns a testnet USDC-equivalent token deployment.
    pub fn testnet_usdc() -> Self {
        MidenUSDC::miden_testnet()
    }

    /// Returns a mainnet USDC-equivalent token deployment.
    pub fn mainnet_usdc() -> Self {
        MidenUSDC::miden_mainnet()
    }
}
