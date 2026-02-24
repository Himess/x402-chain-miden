//! Server-side price tag generation for V2 Miden exact scheme.
//!
//! This module provides functionality for servers to create V2 price tags
//! that clients can use to generate Miden payment authorizations.
//!
//! # Example
//!
//! ```ignore
//! use x402_chain_miden::V2MidenExact;
//! use x402_chain_miden::chain::MidenTokenDeployment;
//!
//! let usdc = MidenTokenDeployment::testnet_usdc();
//! let price_tag = V2MidenExact::price_tag(
//!     "0x1234abcd...".parse().unwrap(),
//!     usdc.amount(1_000_000),
//! );
//! ```

use x402_types::chain::ChainId;
use x402_types::proto::v2;

use crate::V2MidenExact;
use crate::chain::{MidenAccountAddress, MidenDeployedTokenAmount};
use crate::v2_miden_exact::ExactScheme;

impl V2MidenExact {
    /// Creates a V2 price tag for a Miden payment.
    ///
    /// This generates a price tag that specifies the payment requirements
    /// for a resource. The price tag uses CAIP-2 chain IDs (e.g., `miden:testnet`)
    /// and identifies the token by its faucet account ID.
    ///
    /// # Parameters
    ///
    /// - `pay_to`: The recipient's Miden account address
    /// - `asset`: The token deployment and amount required
    ///
    /// # Returns
    ///
    /// A [`v2::PriceTag`] that can be included in a `PaymentRequired` response.
    #[allow(dead_code)]
    pub fn price_tag(pay_to: MidenAccountAddress, asset: MidenDeployedTokenAmount) -> v2::PriceTag {
        let chain_id: ChainId = asset.token.chain_reference.clone().into();
        let requirements = v2::PaymentRequirements {
            scheme: ExactScheme.to_string(),
            pay_to: pay_to.to_string(),
            asset: asset.token.faucet_id.to_string(),
            network: chain_id,
            amount: asset.amount.to_string(),
            max_timeout_seconds: 300,
            extra: None,
        };
        v2::PriceTag {
            requirements,
            enricher: None,
        }
    }
}
