//! Miden blockchain support for the x402 payment protocol.
//!
//! This crate provides implementations of the x402 payment protocol for the
//! Miden ZK rollup. It uses CAIP-2 chain identifiers (`miden:testnet`, `miden:mainnet`)
//! and implements the V2 "exact" payment scheme based on P2ID (Pay-to-ID) notes.
//!
//! # Architecture
//!
//! Unlike EVM chains that use `transferWithAuthorization` (ERC-3009) for gasless
//! token transfers, Miden uses a note-based model:
//!
//! 1. **Client** creates a P2ID note transferring assets to the recipient
//! 2. **Client** executes and proves the transaction locally (STARK proof)
//! 3. **Facilitator** verifies the STARK proof and submits the proven transaction
//! 4. **Settlement** occurs when the Miden network includes the transaction in a block
//!
//! # Feature Flags
//!
//! - `server` - Server-side price tag generation
//! - `client` - Client-side payment signing (P2ID note creation + proving)
//! - `facilitator` - Facilitator-side payment verification and settlement
//! - `miden-native` - Real STARK proof verification using `miden-tx` and `miden-protocol`
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
//!     "0x1234abcd...",  // recipient Miden account ID (hex)
//!     token.amount(1_000_000),  // 1 USDC (6 decimals)
//! );
//! ```
//!
//! ## Client: Signing a Payment
//!
//! ```ignore
//! use x402_chain_miden::V2MidenExactClient;
//!
//! let client = V2MidenExactClient::new(signer);
//! let candidates = client.accept(&payment_required);
//! ```

pub mod chain;
pub mod privacy;
pub mod v2_miden_exact;

mod networks;
pub use networks::*;

pub use v2_miden_exact::V2MidenExact;

#[cfg(feature = "client")]
pub use v2_miden_exact::client::V2MidenExactClient;

#[cfg(all(feature = "client", feature = "miden-client-native"))]
pub use v2_miden_exact::client::MidenClientSigner;
