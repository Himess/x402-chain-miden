//! Standalone x402 facilitator server for the Miden blockchain.
//!
//! This binary runs an HTTP server that provides payment verification and
//! settlement services for x402 payments on the Miden ZK rollup.
//!
//! # Endpoints
//!
//! - `POST /verify`    - Verify a payment payload
//! - `POST /settle`    - Settle a payment on-chain
//! - `GET  /supported` - List supported payment kinds
//! - `GET  /health`    - Health check
//! - `GET  /metrics`   - Prometheus-format metrics
//!
//! # Configuration
//!
//! Set the following environment variables:
//!
//! - `PORT`            - Server port (default: 4020)
//! - `HOST`            - Bind address (default: 0.0.0.0)
//! - `MIDEN_RPC_URL`   - Miden node RPC URL (default: https://rpc.testnet.miden.io)
//! - `MIDEN_NETWORK`   - Network: "testnet" or "mainnet" (default: testnet)

use axum::extract::{DefaultBodyLimit, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::env;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use axum::error_handling::HandleErrorLayer;
use tower::ServiceBuilder;
use tower::buffer::BufferLayer;
use tower::limit::RateLimitLayer;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use x402_chain_miden::chain::{MidenChainConfig, MidenChainProvider, MidenChainReference};
use x402_chain_miden::v2_miden_exact::facilitator::V2MidenExactFacilitator;
use x402_types::chain::ChainProviderOps;
use x402_types::proto;
use x402_types::scheme::X402SchemeFacilitator;

/// Simple atomic counters for Prometheus metrics.
struct Metrics {
    verify_requests_total: AtomicU64,
    settle_requests_total: AtomicU64,
    verify_errors_total: AtomicU64,
    settle_errors_total: AtomicU64,
    // TODO: Add histogram support for verify_duration_seconds / settle_duration_seconds
    // using the `metrics` + `metrics-exporter-prometheus` crates.
}

impl Metrics {
    fn new() -> Self {
        Self {
            verify_requests_total: AtomicU64::new(0),
            settle_requests_total: AtomicU64::new(0),
            verify_errors_total: AtomicU64::new(0),
            settle_errors_total: AtomicU64::new(0),
        }
    }
}

/// Shared application state.
struct AppState {
    facilitator: V2MidenExactFacilitator,
    faucet_id: String,
    metrics: Metrics,
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
    let faucet_id = env::var("FAUCET_ID")
        .unwrap_or_else(|_| "0x37d5977a8e16d8205a360820f0230f".to_string());

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

    let facilitator = V2MidenExactFacilitator::new(provider);
    let state = Arc::new(AppState {
        facilitator,
        faucet_id,
        metrics: Metrics::new(),
    });

    // Rate-limited routes for /verify and /settle: 100 requests per 60 seconds.
    // HandleErrorLayer converts tower errors into HTTP 429 responses.
    // BufferLayer wraps the non-Clone RateLimit service so axum can clone handlers.
    let rate_limited_routes = Router::new()
        .route("/verify", post(verify_handler))
        .route("/settle", post(settle_handler))
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
    }))
}

async fn health_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.facilitator.supported().await {
        Ok(response) => match serde_json::to_value(response) {
            Ok(mut value) => {
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("faucetId".to_string(), serde_json::json!(state.faucet_id));
                }
                (StatusCode::OK, Json(value))
            }
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("serialization error: {e}") })),
            ),
        },
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn supported_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.facilitator.supported().await {
        Ok(response) => match serde_json::to_value(response) {
            Ok(value) => (StatusCode::OK, Json(value)),
            Err(e) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": format!("serialization error: {e}") })),
            ),
        },
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn verify_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    state.metrics.verify_requests_total.fetch_add(1, Ordering::Relaxed);

    let request = match serde_json::from_value::<proto::VerifyRequest>(body) {
        Ok(req) => req,
        Err(e) => {
            state.metrics.verify_errors_total.fetch_add(1, Ordering::Relaxed);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_request",
                    "message": e.to_string(),
                })),
            );
        }
    };

    match state.facilitator.verify(&request).await {
        Ok(response) => match serde_json::to_value(response) {
            Ok(value) => (StatusCode::OK, Json(value)),
            Err(e) => {
                state.metrics.verify_errors_total.fetch_add(1, Ordering::Relaxed);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("serialization error: {e}") })),
                )
            }
        },
        Err(e) => {
            state.metrics.verify_errors_total.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(error = %e, "Verify failed");
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "verification_failed",
                    "message": e.to_string(),
                })),
            )
        }
    }
}

async fn settle_handler(
    State(state): State<Arc<AppState>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    state.metrics.settle_requests_total.fetch_add(1, Ordering::Relaxed);

    let request = match serde_json::from_value::<proto::SettleRequest>(body) {
        Ok(req) => req,
        Err(e) => {
            state.metrics.settle_errors_total.fetch_add(1, Ordering::Relaxed);
            return (
                StatusCode::BAD_REQUEST,
                Json(serde_json::json!({
                    "error": "invalid_request",
                    "message": e.to_string(),
                })),
            );
        }
    };

    match state.facilitator.settle(&request).await {
        Ok(response) => match serde_json::to_value(response) {
            Ok(value) => (StatusCode::OK, Json(value)),
            Err(e) => {
                state.metrics.settle_errors_total.fetch_add(1, Ordering::Relaxed);
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(serde_json::json!({ "error": format!("serialization error: {e}") })),
                )
            }
        },
        Err(e) => {
            state.metrics.settle_errors_total.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(error = %e, "Settle failed");
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(serde_json::json!({
                    "error": "settlement_failed",
                    "message": e.to_string(),
                })),
            )
        }
    }
}

/// Returns Prometheus-format metrics as plain text.
///
/// Tracks basic request counts and error counts. Duration histograms
/// are left as a TODO for a future iteration using the `metrics` crate.
async fn metrics_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let verify_total = state.metrics.verify_requests_total.load(Ordering::Relaxed);
    let settle_total = state.metrics.settle_requests_total.load(Ordering::Relaxed);
    let verify_errors = state.metrics.verify_errors_total.load(Ordering::Relaxed);
    let settle_errors = state.metrics.settle_errors_total.load(Ordering::Relaxed);

    let body = format!(
        "# HELP verify_requests_total Total number of verify requests received.\n\
         # TYPE verify_requests_total counter\n\
         verify_requests_total {verify_total}\n\
         # HELP settle_requests_total Total number of settle requests received.\n\
         # TYPE settle_requests_total counter\n\
         settle_requests_total {settle_total}\n\
         # HELP verify_errors_total Total number of verify errors.\n\
         # TYPE verify_errors_total counter\n\
         verify_errors_total {verify_errors}\n\
         # HELP settle_errors_total Total number of settle errors.\n\
         # TYPE settle_errors_total counter\n\
         settle_errors_total {settle_errors}\n\
         # TODO: Add verify_duration_seconds and settle_duration_seconds histograms\n"
    );

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}
