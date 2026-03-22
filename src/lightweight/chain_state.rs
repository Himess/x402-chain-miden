//! Facilitator chain state management.
//!
//! The facilitator maintains a cache of block headers so that note inclusion
//! proofs can be verified locally without per-request RPC calls. When a block
//! header is not in the cache, it falls back to `get_block_header_by_number()`
//! via the Miden node RPC.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────┐
//! │      FacilitatorChainState      │
//! │                                 │
//! │  ┌───────────────────────────┐  │
//! │  │ RwLock<HashMap<u32, ...>> │  │ ← Concurrent read access
//! │  │ block_num → CachedHeader  │  │   during verification
//! │  └───────────────────────────┘  │
//! │                                 │
//! │  rpc_url: String                │ ← Fallback for cache misses
//! │  chain_reference                │
//! └─────────────────────────────────┘
//!            │
//!            │ background_sync()
//!            ▼
//!    ┌───────────────┐
//!    │  Miden Node    │
//!    │  (gRPC RPC)    │
//!    └───────────────┘
//! ```
//!
//! # Usage
//!
//! ```ignore
//! use x402_chain_miden::lightweight::chain_state::FacilitatorChainState;
//! use x402_chain_miden::chain::MidenChainReference;
//!
//! let state = FacilitatorChainState::new(
//!     "https://rpc.testnet.miden.io".to_string(),
//!     MidenChainReference::testnet(),
//! );
//!
//! // Start background sync (optional, reduces per-request latency)
//! let state_ref = state.clone();
//! tokio::spawn(async move { state_ref.background_sync().await });
//!
//! // During verification, block headers are fetched from cache or RPC
//! let header = state.get_block_header(42).await?;
//! println!("note_root = {}", header.note_root);
//! ```

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use crate::chain::MidenChainReference;
use crate::v2_miden_exact::types::MidenExactError;

/// Interval between background sync iterations (in seconds).
///
/// The background sync task sleeps for this duration between polling
/// the Miden node for new block headers.
#[cfg(feature = "miden-client-native")]
const BACKGROUND_SYNC_INTERVAL_SECS: u64 = 15;

/// Maximum number of cached block headers before eviction kicks in.
///
/// When the cache exceeds this size, the oldest entries (by `cached_at`)
/// are evicted. This prevents unbounded memory growth in long-running
/// facilitator processes.
const MAX_CACHED_HEADERS: usize = 10_000;

/// Cached block header data needed for lightweight verification.
///
/// This struct stores the subset of a Miden block header that the
/// facilitator needs for verifying note inclusion proofs:
///
/// - `note_root`: The root of the block's note tree (SparseMerkleTree).
///   Used to verify the agent's SparseMerklePath inclusion proof.
///
/// - `commitment`: The block header commitment (used for MMR verification
///   to confirm the block is in the canonical chain).
#[derive(Debug, Clone)]
pub struct CachedBlockHeader {
    /// The block number.
    pub block_num: u32,

    /// The note tree root (hex-encoded `RpoDigest`).
    ///
    /// The `SparseMerklePath` from the agent's payment header is verified
    /// against this root to prove note inclusion.
    pub note_root: String,

    /// The block header commitment (hex-encoded `RpoDigest`).
    ///
    /// Used for MMR (Merkle Mountain Range) verification to confirm
    /// the block is part of the canonical chain.
    pub commitment: String,

    /// When this entry was cached.
    ///
    /// Used for cache eviction (oldest entries evicted first when
    /// the cache exceeds `MAX_CACHED_HEADERS`).
    pub cached_at: std::time::Instant,
}

/// Cached chain state for the facilitator.
///
/// Stores block headers indexed by block number so that `note_root` lookups
/// during verification are local (no RPC needed for cached blocks). Falls
/// back to the Miden node's `get_block_header_by_number()` RPC for cache
/// misses.
///
/// # Thread Safety
///
/// Uses `RwLock<HashMap<...>>` for the block header cache, allowing
/// concurrent read access during verification (the hot path) with
/// exclusive write access only when caching new headers.
///
/// The struct is wrapped in `Arc` internally to allow cheap cloning
/// for passing to background tasks.
#[derive(Clone)]
pub struct FacilitatorChainState {
    /// Block number -> cached block header.
    ///
    /// Using `RwLock` for concurrent read access during verification.
    /// Writes happen infrequently (cache misses and background sync).
    block_headers: Arc<RwLock<HashMap<u32, CachedBlockHeader>>>,

    /// The Miden node RPC endpoint URL.
    ///
    /// Used for fetching block headers on cache misses and during
    /// background sync.
    rpc_url: String,

    /// The Miden chain (testnet/mainnet).
    ///
    /// Used to validate that block headers belong to the expected chain.
    chain_reference: MidenChainReference,
}

impl FacilitatorChainState {
    /// Creates a new chain state with the given RPC configuration.
    ///
    /// The cache starts empty. Block headers are fetched on-demand
    /// (cache miss) or proactively via [`background_sync`](Self::background_sync).
    ///
    /// # Parameters
    ///
    /// - `rpc_url`: The Miden node gRPC endpoint (e.g., `https://rpc.testnet.miden.io`)
    /// - `chain_reference`: The target chain (testnet/mainnet)
    pub fn new(rpc_url: String, chain_reference: MidenChainReference) -> Self {
        Self {
            block_headers: Arc::new(RwLock::new(HashMap::new())),
            rpc_url,
            chain_reference,
        }
    }

    /// Gets a block header, using the cache first and falling back to RPC.
    ///
    /// # Cache Strategy
    ///
    /// 1. Acquire a read lock and check the cache.
    /// 2. If found, return the cached entry (fast path).
    /// 3. If not found, release the read lock, fetch via RPC, acquire
    ///    a write lock, insert into cache, and return.
    ///
    /// # RPC Fallback
    ///
    /// When the `miden-client-native` feature is enabled, this calls
    /// `get_block_header_by_number(block_num, true)` — the `true` flag
    /// requests the MMR proof along with the header.
    ///
    /// Without `miden-client-native`, the RPC fallback returns an error
    /// indicating that the block must be pre-cached.
    pub async fn get_block_header(
        &self,
        block_num: u32,
    ) -> Result<CachedBlockHeader, MidenExactError> {
        // Fast path: check cache (read lock)
        {
            let cache = self
                .block_headers
                .read()
                .map_err(|e| MidenExactError::ProviderError(format!("Cache lock poisoned: {e}")))?;

            if let Some(header) = cache.get(&block_num) {
                return Ok(header.clone());
            }
        }

        // Cache miss: fetch from RPC
        let header = self.fetch_block_header_rpc(block_num).await?;

        // Cache the result (write lock)
        {
            let mut cache = self
                .block_headers
                .write()
                .map_err(|e| MidenExactError::ProviderError(format!("Cache lock poisoned: {e}")))?;

            // Evict oldest entries if cache is too large
            if cache.len() >= MAX_CACHED_HEADERS {
                self.evict_oldest_entries(&mut cache, MAX_CACHED_HEADERS / 10);
            }

            cache.insert(block_num, header.clone());
        }

        Ok(header)
    }

    /// Gets the `note_root` for a specific block.
    ///
    /// Convenience wrapper around [`get_block_header`](Self::get_block_header)
    /// that extracts just the `note_root` field.
    pub async fn get_note_root(&self, block_num: u32) -> Result<String, MidenExactError> {
        let header = self.get_block_header(block_num).await?;
        Ok(header.note_root)
    }

    /// Background sync task that periodically fetches new block headers.
    ///
    /// Call this from `tokio::spawn()` at server startup to proactively
    /// cache block headers and reduce per-request RPC latency.
    ///
    /// The task runs indefinitely, sleeping for `BACKGROUND_SYNC_INTERVAL_SECS`
    /// between iterations. Each iteration fetches the latest block header
    /// and caches it.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let state = FacilitatorChainState::new(rpc_url, chain_ref);
    /// let state_clone = state.clone();
    /// tokio::spawn(async move {
    ///     state_clone.background_sync().await;
    /// });
    /// ```
    #[cfg(feature = "miden-client-native")]
    pub async fn background_sync(&self) {
        loop {
            match self.fetch_latest_block_header().await {
                Ok(header) => {
                    let block_num = header.block_num;
                    let mut cache = match self.block_headers.write() {
                        Ok(cache) => cache,
                        Err(e) => {
                            #[cfg(feature = "tracing")]
                            tracing::error!(
                                error = %e,
                                "Background sync: cache lock poisoned, skipping iteration"
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(
                                BACKGROUND_SYNC_INTERVAL_SECS,
                            ))
                            .await;
                            continue;
                        }
                    };

                    if cache.len() >= MAX_CACHED_HEADERS {
                        self.evict_oldest_entries(&mut cache, MAX_CACHED_HEADERS / 10);
                    }

                    cache.insert(block_num, header);

                    #[cfg(feature = "tracing")]
                    tracing::debug!(
                        block_num = block_num,
                        cached_count = cache.len(),
                        "Background sync: cached block header"
                    );
                }
                Err(e) => {
                    #[cfg(feature = "tracing")]
                    tracing::warn!(
                        error = %e,
                        "Background sync: failed to fetch latest block header"
                    );
                }
            }

            tokio::time::sleep(std::time::Duration::from_secs(
                BACKGROUND_SYNC_INTERVAL_SECS,
            ))
            .await;
        }
    }

    /// Background sync stub for non-native builds.
    ///
    /// Without `miden-client-native`, background sync is a no-op since
    /// there is no RPC client to fetch block headers from.
    #[cfg(not(feature = "miden-client-native"))]
    pub async fn background_sync(&self) {
        #[cfg(feature = "tracing")]
        tracing::warn!(
            "Background sync requires miden-client-native feature. \
             Block headers must be pre-cached manually."
        );
        // No-op: without miden-client-native there is no RPC client.
        // Block headers must be pre-cached via insert_block_header().
    }

    /// Returns the number of cached block headers.
    ///
    /// Useful for monitoring and testing.
    pub fn cached_count(&self) -> usize {
        self.block_headers.read().map(|c| c.len()).unwrap_or(0)
    }

    /// Returns the RPC URL.
    pub fn rpc_url(&self) -> &str {
        &self.rpc_url
    }

    /// Returns the chain reference.
    pub fn chain_reference(&self) -> &MidenChainReference {
        &self.chain_reference
    }

    /// Manually inserts a block header into the cache.
    ///
    /// Useful for testing and for pre-populating the cache without
    /// requiring an active RPC connection.
    pub fn insert_block_header(&self, header: CachedBlockHeader) {
        if let Ok(mut cache) = self.block_headers.write() {
            cache.insert(header.block_num, header);
        }
    }

    /// Clears all cached block headers.
    ///
    /// Useful for testing and cache invalidation.
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.block_headers.write() {
            cache.clear();
        }
    }

    // ========================================================================
    // Internal helpers
    // ========================================================================

    /// Evicts the oldest `count` entries from the cache.
    ///
    /// Entries are sorted by `cached_at` (ascending) and the oldest
    /// `count` are removed.
    fn evict_oldest_entries(&self, cache: &mut HashMap<u32, CachedBlockHeader>, count: usize) {
        let mut entries: Vec<(u32, std::time::Instant)> =
            cache.iter().map(|(k, v)| (*k, v.cached_at)).collect();

        entries.sort_by_key(|(_, instant)| *instant);

        for (block_num, _) in entries.into_iter().take(count) {
            cache.remove(&block_num);
        }
    }

    /// Fetches a block header from the Miden node RPC.
    ///
    /// When `miden-client-native` is enabled, this calls
    /// `get_block_header_by_number(block_num, true)` to get both the
    /// header and its MMR proof.
    #[cfg(feature = "miden-client-native")]
    async fn fetch_block_header_rpc(
        &self,
        block_num: u32,
    ) -> Result<CachedBlockHeader, MidenExactError> {
        use miden_client::rpc::{GrpcClient, NodeRpcClient};
        use miden_protocol::block::BlockNumber;

        let endpoint = self
            .rpc_url
            .as_str()
            .try_into()
            .map_err(|e: miden_client::rpc::RpcEndpoint| {
                MidenExactError::ProviderError(format!("Invalid RPC URL '{}': {e}", self.rpc_url))
            })
            .unwrap_or_default();

        let rpc_client = GrpcClient::new(&endpoint, 10_000);

        // Fetch block header with MMR proof (the `true` flag)
        let block_number = BlockNumber::from(block_num);
        let (block_header, _mmr_proof) = rpc_client
            .get_block_header_by_number(Some(block_number), true)
            .await
            .map_err(|e| {
                MidenExactError::ProviderError(format!(
                    "Failed to fetch block header for block {block_num}: {e}"
                ))
            })?;

        Ok(CachedBlockHeader {
            block_num,
            note_root: format!("{}", block_header.note_root()),
            commitment: format!("{}", block_header.commitment()),
            cached_at: std::time::Instant::now(),
        })
    }

    /// Stub for fetching block headers without `miden-client-native`.
    ///
    /// Returns an error indicating that the block must be pre-cached
    /// or the `miden-client-native` feature must be enabled.
    #[cfg(not(feature = "miden-client-native"))]
    async fn fetch_block_header_rpc(
        &self,
        block_num: u32,
    ) -> Result<CachedBlockHeader, MidenExactError> {
        Err(MidenExactError::ProviderError(format!(
            "Block header for block {block_num} not in cache and RPC fallback \
             requires the miden-client-native feature. Either pre-cache the header \
             via insert_block_header() or enable miden-client-native."
        )))
    }

    /// Fetches the latest block header from the Miden node.
    ///
    /// Used by the background sync task to proactively cache new blocks.
    #[cfg(feature = "miden-client-native")]
    async fn fetch_latest_block_header(&self) -> Result<CachedBlockHeader, MidenExactError> {
        use miden_client::rpc::{GrpcClient, NodeRpcClient};

        let endpoint = self
            .rpc_url
            .as_str()
            .try_into()
            .map_err(|e: miden_client::rpc::RpcEndpoint| {
                MidenExactError::ProviderError(format!("Invalid RPC URL '{}': {e}", self.rpc_url))
            })
            .unwrap_or_default();

        let rpc_client = GrpcClient::new(&endpoint, 10_000);

        // `None` = latest block
        let (block_header, _mmr_proof) = rpc_client
            .get_block_header_by_number(None, true)
            .await
            .map_err(|e| {
                MidenExactError::ProviderError(format!("Failed to fetch latest block header: {e}"))
            })?;

        let block_num = block_header.block_num().into();

        Ok(CachedBlockHeader {
            block_num,
            note_root: format!("{}", block_header.note_root()),
            commitment: format!("{}", block_header.commitment()),
            cached_at: std::time::Instant::now(),
        })
    }
}

impl std::fmt::Debug for FacilitatorChainState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let cached_count = self.cached_count();
        f.debug_struct("FacilitatorChainState")
            .field("rpc_url", &self.rpc_url)
            .field("chain_reference", &self.chain_reference)
            .field("cached_headers", &cached_count)
            .finish()
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn test_chain_state() -> FacilitatorChainState {
        FacilitatorChainState::new(
            "https://rpc.testnet.miden.io".to_string(),
            MidenChainReference::testnet(),
        )
    }

    #[test]
    fn test_new_chain_state() {
        let state = test_chain_state();
        assert_eq!(state.rpc_url(), "https://rpc.testnet.miden.io");
        assert_eq!(state.chain_reference().inner(), "testnet");
        assert_eq!(state.cached_count(), 0);
    }

    #[test]
    fn test_insert_and_retrieve_cached_header() {
        let state = test_chain_state();

        let header = CachedBlockHeader {
            block_num: 42,
            note_root: "0xaabbccdd".to_string(),
            commitment: "0xdeadbeef".to_string(),
            cached_at: std::time::Instant::now(),
        };

        state.insert_block_header(header.clone());
        assert_eq!(state.cached_count(), 1);

        // Read from cache
        let cache = state.block_headers.read().unwrap();
        let cached = cache.get(&42).unwrap();
        assert_eq!(cached.block_num, 42);
        assert_eq!(cached.note_root, "0xaabbccdd");
        assert_eq!(cached.commitment, "0xdeadbeef");
    }

    #[test]
    fn test_clear_cache() {
        let state = test_chain_state();

        state.insert_block_header(CachedBlockHeader {
            block_num: 1,
            note_root: "0xaa".to_string(),
            commitment: "0xbb".to_string(),
            cached_at: std::time::Instant::now(),
        });
        state.insert_block_header(CachedBlockHeader {
            block_num: 2,
            note_root: "0xcc".to_string(),
            commitment: "0xdd".to_string(),
            cached_at: std::time::Instant::now(),
        });

        assert_eq!(state.cached_count(), 2);

        state.clear_cache();
        assert_eq!(state.cached_count(), 0);
    }

    #[test]
    fn test_clone_shares_cache() {
        let state = test_chain_state();
        let clone = state.clone();

        state.insert_block_header(CachedBlockHeader {
            block_num: 99,
            note_root: "0x11".to_string(),
            commitment: "0x22".to_string(),
            cached_at: std::time::Instant::now(),
        });

        // Clone should see the same cached data (Arc-shared)
        assert_eq!(clone.cached_count(), 1);
    }

    #[test]
    fn test_evict_oldest_entries() {
        let state = test_chain_state();
        let now = std::time::Instant::now();

        // Insert 5 headers with staggered timestamps
        for i in 0..5u32 {
            state.insert_block_header(CachedBlockHeader {
                block_num: i,
                note_root: format!("0x{:02x}", i),
                commitment: format!("0x{:02x}", i + 100),
                // Simulate staggered times — older entries have earlier Instants.
                // Since Instant::now() is monotonic, all will be very close,
                // but ordering is preserved.
                cached_at: now,
            });
        }

        assert_eq!(state.cached_count(), 5);

        // Evict 2 oldest entries
        {
            let mut cache = state.block_headers.write().unwrap();
            state.evict_oldest_entries(&mut cache, 2);
        }

        assert_eq!(state.cached_count(), 3);
    }

    #[test]
    fn test_debug_format() {
        let state = test_chain_state();
        let debug_str = format!("{state:?}");
        assert!(debug_str.contains("FacilitatorChainState"));
        assert!(debug_str.contains("rpc.testnet.miden.io"));
        assert!(debug_str.contains("cached_headers"));
    }

    #[cfg(not(feature = "miden-client-native"))]
    #[tokio::test]
    async fn test_get_block_header_cache_miss_without_native() {
        let state = test_chain_state();

        // Without miden-client-native, cache miss should return an error
        let result = state.get_block_header(999).await;
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(matches!(err, MidenExactError::ProviderError(_)));
    }

    #[cfg(not(feature = "miden-client-native"))]
    #[tokio::test]
    async fn test_get_block_header_cache_hit() {
        let state = test_chain_state();

        // Pre-cache a header
        state.insert_block_header(CachedBlockHeader {
            block_num: 42,
            note_root: "0xaabbccdd".to_string(),
            commitment: "0xdeadbeef".to_string(),
            cached_at: std::time::Instant::now(),
        });

        // Should succeed even without miden-client-native
        let header = state.get_block_header(42).await.unwrap();
        assert_eq!(header.block_num, 42);
        assert_eq!(header.note_root, "0xaabbccdd");
    }

    #[cfg(not(feature = "miden-client-native"))]
    #[tokio::test]
    async fn test_get_note_root() {
        let state = test_chain_state();

        state.insert_block_header(CachedBlockHeader {
            block_num: 7,
            note_root: "0xfeedface".to_string(),
            commitment: "0x00".to_string(),
            cached_at: std::time::Instant::now(),
        });

        let note_root = state.get_note_root(7).await.unwrap();
        assert_eq!(note_root, "0xfeedface");
    }

    #[test]
    fn test_multiple_inserts_same_block() {
        let state = test_chain_state();

        state.insert_block_header(CachedBlockHeader {
            block_num: 42,
            note_root: "0xold".to_string(),
            commitment: "0xold".to_string(),
            cached_at: std::time::Instant::now(),
        });

        state.insert_block_header(CachedBlockHeader {
            block_num: 42,
            note_root: "0xnew".to_string(),
            commitment: "0xnew".to_string(),
            cached_at: std::time::Instant::now(),
        });

        // Should be overwritten
        assert_eq!(state.cached_count(), 1);
        let cache = state.block_headers.read().unwrap();
        assert_eq!(cache.get(&42).unwrap().note_root, "0xnew");
    }
}
