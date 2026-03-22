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

/// Environment variable name for overriding the testnet faucet ID at runtime.
///
/// Set `MIDEN_TESTNET_FAUCET_ID=0x...` to use a custom faucet on testnet.
/// This is useful for testing with your own faucet deployment.
pub const TESTNET_FAUCET_ENV: &str = "MIDEN_TESTNET_FAUCET_ID";

/// Default testnet faucet ID.
///
/// This is the public fungible-token faucet deployed on Miden testnet.
/// Faucet metadata: <https://faucet-api-testnet-miden.eu-central-8.gateway.fm/get_metadata>
/// Faucet UI: <https://faucet.testnet.miden.io>
///
/// Note: This faucet ID may change across testnet resets. Override at runtime
/// via the `MIDEN_TESTNET_FAUCET_ID` environment variable if needed.
const DEFAULT_TESTNET_FAUCET_HEX: &str = "0x37d5977a8e16d8205a360820f0230f";

fn testnet_faucet_id() -> MidenAccountAddress {
    std::env::var(TESTNET_FAUCET_ENV)
        .ok()
        .and_then(|v| v.parse::<MidenAccountAddress>().ok())
        .unwrap_or_else(|| {
            DEFAULT_TESTNET_FAUCET_HEX
                .parse()
                .expect("default testnet faucet hex is valid")
        })
}

impl KnownNetworkMiden<MidenTokenDeployment> for MidenUSDC {
    fn miden_testnet() -> MidenTokenDeployment {
        MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            faucet_id: testnet_faucet_id(),
            decimals: 6,
        }
    }

    fn miden_mainnet() -> MidenTokenDeployment {
        MidenTokenDeployment {
            chain_reference: MidenChainReference::mainnet(),
            // Mainnet faucet ID will be set at mainnet launch.
            // Using a distinctive placeholder that will fail loudly if accidentally used.
            // All 0xFF bytes are an invalid account ID in Miden, so any attempt to
            // transact with this placeholder will produce an immediate error rather
            // than a silent mis-payment.
            faucet_id: MidenAccountAddress::from_bytes(&[0xFF; 15])
                .expect("15-byte placeholder is always valid"),
            decimals: 6,
        }
    }
}

impl MidenTokenDeployment {
    /// Convenience alias for [`MidenUSDC::miden_testnet()`].
    ///
    /// Returns a testnet USDC-equivalent token deployment. Delegates to
    /// [`KnownNetworkMiden::miden_testnet()`] on [`MidenUSDC`].
    pub fn testnet_usdc() -> Self {
        MidenUSDC::miden_testnet()
    }

    /// Convenience alias for [`MidenUSDC::miden_mainnet()`].
    ///
    /// Returns a mainnet USDC-equivalent token deployment. Delegates to
    /// [`KnownNetworkMiden::miden_mainnet()`] on [`MidenUSDC`].
    pub fn mainnet_usdc() -> Self {
        MidenUSDC::miden_mainnet()
    }
}
