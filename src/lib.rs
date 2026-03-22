//! Miden blockchain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for the
//! Miden ZK rollup. It uses CAIP-2 chain identifiers (`miden:testnet`, `miden:mainnet`)
//! and implements the V2 "exact" payment scheme based on P2ID (Pay-to-ID) notes.
//!
//! # Architecture (Lightweight / bobbinth's design)
//!
//! Unlike the legacy STARK-proof-based flow, the lightweight design has the
//! agent submit the transaction directly to the Miden network and send only
//! a compact inclusion proof (~200 bytes) to the facilitator:
//!
//! 1. **Server** generates a payment requirement with a `recipient_digest`
//! 2. **Agent** creates a P2ID note, proves it, and submits to the network
//! 3. **Agent** sends `{note_id, block_num, inclusion_proof}` to the server
//! 4. **Server** verifies `NoteId` matches and the Merkle inclusion proof is valid
//!
//! # Feature Flags
//!
//! - `server` - Server-side price tag generation
//! - `client` - Client-side lightweight payment creation
//! - `facilitator` - Facilitator-side chain provider and lightweight verification
//! - `miden-native` - Miden protocol types using `miden-protocol`
//! - `miden-client-native` - Full miden-client integration (includes `miden-native`)
//!
//! # Usage
//!
//! ## Server: Creating a Price Tag
//!
//! ```ignore
//! use x402_chain_miden::V2MidenExact;
//! use x402_chain_miden::chain::MidenTokenDeployment;
//!
//! let token = MidenTokenDeployment::testnet_usdc();
//! let price_tag = V2MidenExact::price_tag(
//!     "0x1234abcd...".parse().unwrap(),
//!     token.amount(1_000_000),  // 1 USDC (6 decimals)
//! );
//! ```

pub mod chain;
pub mod lightweight;
pub mod v2_miden_exact;

mod networks;
pub use networks::*;

pub use v2_miden_exact::V2MidenExact;

#[cfg(all(feature = "client", feature = "miden-client-native"))]
pub use lightweight::client::LightweightMidenPayer;
