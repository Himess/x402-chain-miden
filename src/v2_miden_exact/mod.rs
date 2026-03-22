//! V2 Miden "exact" payment scheme implementation.
//!
//! This module implements the "exact" payment scheme for the Miden blockchain
//! using the V2 x402 protocol with CAIP-2 chain identifiers.
//!
//! # Payment Model (Lightweight / bobbinth's design)
//!
//! The agent submits the transaction directly to the Miden network and sends
//! only a compact inclusion proof to the server:
//!
//! 1. Server generates a payment requirement with `recipient_digest`
//! 2. Agent creates a P2ID note, proves it, submits to network
//! 3. Agent sends `{note_id, block_num, inclusion_proof}` to server
//! 4. Server verifies NoteId + Merkle inclusion proof
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

pub mod types;
pub use types::*;

use x402_types::scheme::X402SchemeId;

/// The V2 Miden "exact" payment scheme.
///
/// This struct serves as the scheme identifier and factory for creating
/// price tags for Miden payments.
pub struct V2MidenExact;

impl X402SchemeId for V2MidenExact {
    fn namespace(&self) -> &str {
        "miden"
    }

    fn scheme(&self) -> &str {
        ExactScheme.as_ref()
    }
}
