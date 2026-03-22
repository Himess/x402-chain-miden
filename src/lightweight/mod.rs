//! Lightweight payment verification using note inclusion proofs.
//!
//! This module implements bobbinth's design from 0xMiden/node#1796:
//! instead of sending the full `ProvenTransaction` + STARK proof to the
//! facilitator, the agent submits the transaction directly to the Miden
//! network and sends only a lightweight payment header containing the
//! note ID and Merkle inclusion proof (~200 bytes).
//!
//! # Flow
//!
//! ```text
//! Agent                              Server
//!   |                                    |
//!   |-- GET /resource ------------------>|
//!   |<-- 402 {recipient_digest, asset,   |
//!   |         note_tag} ----------------|
//!   |                                    |
//!   | Create P2ID note                   |
//!   | STARK prove + submit to network    |
//!   | sync_state() -> inclusion proof    |
//!   |                                    |
//!   |-- {note_id, block_num,            |
//!   |    note_inclusion_proof} --------->|
//!   |                                    |
//!   |    Server: NoteId == expected?     |
//!   |    SparseMerklePath.verify()       |
//!   |                                    |
//!   |<-- 200 OK -------------------------|
//! ```
//!
//! # Design Details (bobbinth)
//!
//! - The server sends `{recipient_digest, asset, note_tag}` in the 402
//!   response. The `recipient_digest` is computed server-side from a
//!   random `serial_num`, the P2ID script root, and the recipient's
//!   account ID. The `serial_num` itself is **not** sent to the agent
//!   (unless explicitly opted in for nullifier tracking).
//!
//! - The agent creates a P2ID note matching the `recipient_digest` and
//!   `asset`, proves the transaction locally, submits it to the network,
//!   then calls `sync_state()` to obtain an inclusion proof.
//!
//! - The agent sends back `{note_id, block_num, inclusion_proof}` — a
//!   lightweight header of approximately 200 bytes.
//!
//! - The server verifies:
//!   1. `NoteId == hash(recipient_digest, asset_commitment)` — the note
//!      pays the correct recipient with the correct asset.
//!   2. `SparseMerklePath.verify()` — the note is actually included in
//!      the specified block's note tree.
//!
//! # Advantages over STARK-based verification
//!
//! - **Bandwidth**: ~200 bytes vs. ~100 KB for a full `ProvenTransaction`
//! - **Latency**: Merkle path verification is O(log n) hashes vs. STARK
//!   proof verification
//! - **Simplicity**: No need for the server to run the Miden VM verifier

pub mod types;
pub mod server;
pub mod chain_state;
pub mod verification;

#[cfg(feature = "client")]
pub mod client;

pub use types::*;
pub use server::*;
pub use chain_state::{CachedBlockHeader, FacilitatorChainState};

/// Async version of lightweight payment verification that uses
/// [`FacilitatorChainState`] for block header lookups and performs
/// full NoteId reconstruction and SparseMerklePath verification
/// (requires `miden-native` feature).
///
/// See [`verification::verify_lightweight_payment`] for details.
pub use verification::verify_lightweight_payment as verify_lightweight_payment_full;

#[cfg(feature = "client")]
pub use client::*;
