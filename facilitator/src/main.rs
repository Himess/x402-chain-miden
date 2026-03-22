//! Standalone x402 facilitator server for the Miden blockchain.
//!
//! This binary runs an HTTP server that provides lightweight payment
//! verification services for x402 payments on the Miden ZK rollup,
//! using bobbinth's design (note inclusion proof-based verification).
//!
//! # Endpoints
//!
//! - `POST /payment-requirement` - Generate a 402 payment requirement + server context
//! - `POST /verify-lightweight`  - Verify a lightweight payment header (note_id + inclusion proof)
//! - `GET  /`                    - Service info
//! - `GET  /health`              - Health check
//! - `GET  /supported`           - List supported payment kinds
//! - `GET  /metrics`             - Prometheus-format metrics
//!
//! # Configuration
//!
//! Set the following environment variables:
//!
//! - `PORT`            - Server port (default: 4020)
//! - `HOST`            - Bind address (default: 0.0.0.0)
//! - `MIDEN_RPC_URL`   - Miden node RPC URL (default: https://rpc.testnet.miden.io)
//! - `MIDEN_NETWORK`   - Network: "testnet" or "mainnet" (default: testnet)

use axum::error_handling::HandleErrorLayer;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::collections::HashMap;
use std::env;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tower::ServiceBuilder;
use tower::buffer::BufferLayer;
use tower::limit::RateLimitLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use x402_chain_miden::chain::{MidenChainConfig, MidenChainProvider, MidenChainReference};
use x402_chain_miden::lightweight::{
    FacilitatorChainState, PaymentContext,
    server::{DEFAULT_CONTEXT_TIMEOUT_SECS, create_payment_requirement},
    types::LightweightPaymentHeader,
    verify_lightweight_payment_full,
};
use x402_types::chain::{ChainId, ChainProviderOps};

/// Simple atomic counters for Prometheus metrics.
struct Metrics {
    lightweight_verify_requests_total: AtomicU64,
    lightweight_verify_errors_total: AtomicU64,
    payment_requirement_requests_total: AtomicU64,
}

impl Metrics {
    fn new() -> Self {
        Self {
            lightweight_verify_requests_total: AtomicU64::new(0),
            lightweight_verify_errors_total: AtomicU64::new(0),
            payment_requirement_requests_total: AtomicU64::new(0),
        }
    }
}

/// Shared application state.
struct AppState {
    faucet_id: String,
    metrics: Metrics,

    /// In-memory store for pending lightweight payment contexts.
    ///
    /// Maps `context_id` -> `PaymentContext`. Entries are created by
    /// `POST /payment-requirement` and consumed by `POST /verify-lightweight`.
    ///
    /// Per bobbinth's design, the server keeps the `serial_num` and
    /// `recipient_digest` so it can recompute the expected `NoteId`
    /// when the agent returns with the lightweight payment header.
    payment_contexts: RwLock<HashMap<String, PaymentContext>>,

    /// Cached block headers for lightweight verification.
    ///
    /// Used to look up `note_root` when verifying note inclusion proofs
    /// without making per-request RPC calls.
    chain_state: FacilitatorChainState,

    /// The CAIP-2 chain ID (e.g., `miden:testnet`).
    chain_id: ChainId,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing: LOG_LEVEL is used if RUST_LOG is not set
    let log_level = env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(&log_level)),
        )
        .init();

    // Read configuration from environment
    let rpc_url =
        env::var("MIDEN_RPC_URL").unwrap_or_else(|_| "https://rpc.testnet.miden.io".to_string());
    let network = env::var("MIDEN_NETWORK").unwrap_or_else(|_| "testnet".to_string());
    let faucet_id =
        env::var("FAUCET_ID").unwrap_or_else(|_| "0x37d5977a8e16d8205a360820f0230f".to_string());

    // Build Miden provider
    let chain_reference = MidenChainReference::try_from(network.as_str())
        .expect("Invalid MIDEN_NETWORK: must be 'testnet' or 'mainnet'");

    let config = MidenChainConfig {
        chain_reference,
        rpc_url,
    };
    let provider = MidenChainProvider::from_config(&config);

    tracing::info!(
        chain_id = %provider.chain_id(),
        faucet_id = %faucet_id,
        "Miden facilitator starting"
    );

    let chain_id = provider.chain_id();

    // Build chain state for lightweight verification (block header cache)
    let chain_state =
        FacilitatorChainState::new(config.rpc_url.clone(), config.chain_reference.clone());

    // Start background sync for block header caching
    let chain_state_bg = chain_state.clone();
    tokio::spawn(async move {
        chain_state_bg.background_sync().await;
    });

    let state = Arc::new(AppState {
        faucet_id,
        metrics: Metrics::new(),
        payment_contexts: RwLock::new(HashMap::new()),
        chain_state,
        chain_id,
    });

    // Rate-limited routes: 100 requests per 60 seconds.
    // HandleErrorLayer converts tower errors into HTTP 429 responses.
    // BufferLayer wraps the non-Clone RateLimit service so axum can clone handlers.
    let rate_limited_routes = Router::new()
        .route("/payment-requirement", post(payment_requirement_handler))
        .route("/verify-lightweight", post(verify_lightweight_handler))
        .layer(
            ServiceBuilder::new()
                .layer(HandleErrorLayer::new(|err: tower::BoxError| async move {
                    tracing::warn!(error = %err, "Rate limit or buffer error");
                    (
                        StatusCode::TOO_MANY_REQUESTS,
                        Json(serde_json::json!({
                            "error": "rate_limited",
                            "message": "Too many requests. Please try again later.",
                        })),
                    )
                }))
                .layer(BufferLayer::new(256))
                .layer(RateLimitLayer::new(100, Duration::from_secs(60))),
        );

    // Build router: non-rate-limited routes + rate-limited routes
    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/supported", get(supported_handler))
        .route("/metrics", get(metrics_handler))
        .merge(rate_limited_routes)
        .layer(DefaultBodyLimit::max(2 * 1024 * 1024)) // 2 MB
        .layer(CorsLayer::permissive())
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    // BIND_ADDR takes precedence; fall back to HOST:PORT for backward compat
    let bind_address = env::var("BIND_ADDR").unwrap_or_else(|_| {
        let port: u16 = env::var("PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(4020);
        let host = env::var("HOST").unwrap_or_else(|_| "0.0.0.0".to_string());
        format!("{host}:{port}")
    });
    let listener = tokio::net::TcpListener::bind(&bind_address).await?;
    tracing::info!("Listening on {bind_address}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Waits for a Ctrl-C signal to initiate graceful shutdown.
async fn shutdown_signal() {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to install Ctrl-C handler");
    tracing::info!("Shutdown signal received, draining connections...");
}

async fn root_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(serde_json::json!({
        "service": "x402-miden-facilitator",
        "version": env!("CARGO_PKG_VERSION"),
        "chain": "miden",
        "scheme": "exact",
        "faucetId": state.faucet_id,
        "endpoints": {
            "lightweight": ["/payment-requirement", "/verify-lightweight"],
        },
    }))
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let cached_headers = state.chain_state.cached_count();
    let pending_contexts = state.payment_contexts.read().map(|c| c.len()).unwrap_or(0);

    (
        StatusCode::OK,
        Json(serde_json::json!({
            "status": "ok",
            "chain_id": state.chain_id.to_string(),
            "faucetId": state.faucet_id,
            "cached_block_headers": cached_headers,
            "pending_payment_contexts": pending_contexts,
        })),
    )
}

async fn supported_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "kinds": [{
                "x402Version": 2,
                "scheme": "exact",
                "network": state.chain_id.to_string(),
            }],
            "verification": "lightweight",
        })),
    )
}

/// Returns Prometheus-format metrics as plain text.
async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let lw_verify_total = state
        .metrics
        .lightweight_verify_requests_total
        .load(Ordering::Relaxed);
    let lw_verify_errors = state
        .metrics
        .lightweight_verify_errors_total
        .load(Ordering::Relaxed);
    let pr_total = state
        .metrics
        .payment_requirement_requests_total
        .load(Ordering::Relaxed);
    let pending_contexts = state.payment_contexts.read().map(|c| c.len()).unwrap_or(0);
    let cached_headers = state.chain_state.cached_count();

    let body = format!(
        "# HELP lightweight_verify_requests_total Total lightweight verify requests.\n\
         # TYPE lightweight_verify_requests_total counter\n\
         lightweight_verify_requests_total {lw_verify_total}\n\
         # HELP lightweight_verify_errors_total Total lightweight verify errors.\n\
         # TYPE lightweight_verify_errors_total counter\n\
         lightweight_verify_errors_total {lw_verify_errors}\n\
         # HELP payment_requirement_requests_total Total payment requirement requests.\n\
         # TYPE payment_requirement_requests_total counter\n\
         payment_requirement_requests_total {pr_total}\n\
         # HELP pending_payment_contexts Number of pending lightweight payment contexts.\n\
         # TYPE pending_payment_contexts gauge\n\
         pending_payment_contexts {pending_contexts}\n\
         # HELP cached_block_headers Number of cached block headers.\n\
         # TYPE cached_block_headers gauge\n\
         cached_block_headers {cached_headers}\n"
    );

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}

// ============================================================================
// Lightweight payment endpoints (bobbinth's design, 0xMiden/node#1796)
// ============================================================================

/// Request body for `POST /payment-requirement`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct PaymentRequirementRequest {
    /// The recipient's Miden account ID (hex-encoded).
    recipient: String,
    /// The faucet account ID (hex-encoded) for the token.
    asset: String,
    /// The required payment amount in the token's smallest unit.
    amount: u64,
    /// The note tag for efficient filtering (optional, defaults to 0).
    #[serde(default)]
    note_tag: u32,
}

/// Response body for `POST /payment-requirement`.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct PaymentRequirementResponse {
    /// The unique context ID. The resource server must include this when
    /// calling `/verify-lightweight` after the agent submits payment.
    context_id: String,
    /// The lightweight payment requirement to return to the agent.
    requirement: x402_chain_miden::lightweight::types::LightweightPaymentRequirement,
}

/// Generates a lightweight payment requirement and stores the context.
async fn payment_requirement_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PaymentRequirementRequest>,
) -> impl IntoResponse {
    state
        .metrics
        .payment_requirement_requests_total
        .fetch_add(1, Ordering::Relaxed);

    let (requirement, context) = match create_payment_requirement(
        &body.recipient,
        &body.asset,
        body.amount,
        body.note_tag,
        state.chain_id.clone(),
    ) {
        Ok(result) => result,
        Err(e) => {
            tracing::warn!(
                error = %e,
                recipient = %body.recipient,
                "Failed to create payment requirement"
            );
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_request",
                    "message": e,
                })),
            );
        }
    };

    // Generate a unique context ID using cryptographically secure random bytes
    let context_id = {
        let mut id_bytes = [0u8; 16];
        getrandom::getrandom(&mut id_bytes)
            .expect("Failed to generate random bytes for context ID");
        format!("ctx-{}", hex::encode(id_bytes))
    };

    // Store the context
    match state.payment_contexts.write() {
        Ok(mut contexts) => {
            // Prune expired contexts while we have the write lock
            contexts.retain(|_, ctx| !ctx.is_expired(DEFAULT_CONTEXT_TIMEOUT_SECS));
            contexts.insert(context_id.clone(), context);

            tracing::info!(
                context_id = %context_id,
                recipient = %body.recipient,
                asset = %body.asset,
                amount = body.amount,
                pending_contexts = contexts.len(),
                "Created lightweight payment context"
            );
        }
        Err(e) => {
            tracing::error!(error = %e, "Failed to acquire write lock on payment contexts");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal_error",
                    "message": "Failed to store payment context",
                })),
            );
        }
    }

    let response = PaymentRequirementResponse {
        context_id,
        requirement,
    };

    match serde_json::to_value(response) {
        Ok(value) => (StatusCode::OK, Json(value)),
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": format!("serialization error: {e}") })),
        ),
    }
}

/// Request body for `POST /verify-lightweight`.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerifyLightweightRequest {
    /// The payment context ID returned by `/payment-requirement`.
    payment_context_id: String,
    /// The lightweight payment header from the agent.
    payment_header: LightweightPaymentHeader,
}

/// Verifies a lightweight payment header against a stored payment context.
async fn verify_lightweight_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<VerifyLightweightRequest>,
) -> impl IntoResponse {
    state
        .metrics
        .lightweight_verify_requests_total
        .fetch_add(1, Ordering::Relaxed);

    // 1. Prune expired contexts, then look up the requested one.
    //    We take a write lock so we can remove stale entries before lookup.
    let context = match state.payment_contexts.write() {
        Ok(mut contexts) => {
            contexts.retain(|_, ctx| !ctx.is_expired(DEFAULT_CONTEXT_TIMEOUT_SECS));
            match contexts.get(&body.payment_context_id) {
                Some(ctx) => {
                    // Clone the relevant data we need for verification
                    PaymentContext::new(
                        ctx.recipient_digest.clone(),
                        ctx.asset_faucet_id.clone(),
                        ctx.amount,
                        ctx.note_tag,
                        ctx.serial_num.clone(),
                    )
                }
                None => {
                    state
                        .metrics
                        .lightweight_verify_errors_total
                        .fetch_add(1, Ordering::Relaxed);
                    return (
                        StatusCode::NOT_FOUND,
                        Json(serde_json::json!({
                            "error": "context_not_found",
                            "message": format!(
                                "Payment context '{}' not found or expired",
                                body.payment_context_id
                            ),
                        })),
                    );
                }
            }
        }
        Err(e) => {
            state
                .metrics
                .lightweight_verify_errors_total
                .fetch_add(1, Ordering::Relaxed);
            tracing::error!(error = %e, "Failed to acquire write lock on payment contexts");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({
                    "error": "internal_error",
                    "message": "Failed to read payment contexts",
                })),
            );
        }
    };

    // 2. Check expiry before performing full verification
    if context.is_expired(DEFAULT_CONTEXT_TIMEOUT_SECS) {
        state
            .metrics
            .lightweight_verify_errors_total
            .fetch_add(1, Ordering::Relaxed);
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": "context_expired",
                "message": "Payment context has expired",
            })),
        );
    }

    // 3. Verify the lightweight payment using full crypto verification
    //    (NoteId reconstruction + SparseMerklePath + FacilitatorChainState)
    let result =
        verify_lightweight_payment_full(&context, &body.payment_header, &state.chain_state).await;

    match result {
        Ok(response) => {
            // On successful verification, remove the context to prevent replay
            if response.valid {
                if let Ok(mut contexts) = state.payment_contexts.write() {
                    contexts.remove(&body.payment_context_id);
                    tracing::info!(
                        context_id = %body.payment_context_id,
                        note_id = %response.note_id,
                        block_num = response.block_num,
                        "Lightweight payment verified and context consumed"
                    );
                }
            }

            match serde_json::to_value(&response) {
                Ok(value) => (StatusCode::OK, Json(value)),
                Err(e) => (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("serialization error: {e}") })),
                ),
            }
        }
        Err(e) => {
            state
                .metrics
                .lightweight_verify_errors_total
                .fetch_add(1, Ordering::Relaxed);
            tracing::warn!(
                error = %e,
                context_id = %body.payment_context_id,
                "Lightweight verify failed"
            );
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "lightweight_verification_failed",
                    "message": e.to_string(),
                })),
            )
        }
    }
}
