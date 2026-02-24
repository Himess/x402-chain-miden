//! V2 Miden "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for the Miden blockchain
//! using the V2 x402 protocol with CAIP-2 chain identifiers.
//!
//! # Payment Model
//!
//! Unlike EVM chains that use `transferWithAuthorization` (ERC-3009) for
//! gasless token transfers, Miden uses a note-based payment model:
//!
//! 1. Client creates a P2ID (Pay-to-ID) note directing assets to the recipient
//! 2. Client executes the transaction locally in the Miden VM
//! 3. Client generates a STARK proof of correct execution
//! 4. The serialized `ProvenTransaction` is sent as the payment payload
//! 5. Facilitator verifies the proof and submits to the Miden network
//!
//! # Features
//!
//! - P2ID note-based payments (Miden's equivalent of token transfers)
//! - STARK proof verification for payment validity
//! - Client-side execution and proving (no facilitator-side signing needed)
//!
//! # Usage
//!
//! ```ignore
//! use x402_chain_miden::v2_miden_exact::V2MidenExact;
//! use x402_chain_miden::chain::MidenTokenDeployment;
//!
//! let usdc = MidenTokenDeployment::testnet_usdc();
//! let price_tag = V2MidenExact::price_tag(
//!     "0x1234abcd...".parse().unwrap(),
//!     usdc.amount(1_000_000),
//! );
//! ```

#[cfg(feature = "server")]
pub mod server;
#[cfg(feature = "server")]
#[allow(unused_imports)]
pub use server::*;

#[cfg(feature = "facilitator")]
pub mod facilitator;
#[cfg(feature = "facilitator")]
pub use facilitator::*;

#[cfg(feature = "client")]
pub mod client;
#[cfg(feature = "client")]
pub use client::*;

pub mod types;
pub use types::*;

use x402_types::scheme::X402SchemeId;

/// The V2 Miden "exact" payment scheme.
///
/// This struct serves as the scheme identifier and factory for creating
/// price tags, clients, and facilitators for Miden payments.
pub struct V2MidenExact;

impl X402SchemeId for V2MidenExact {
    fn namespace(&self) -> &str {
        "miden"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
