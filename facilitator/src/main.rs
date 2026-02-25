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
//!
//! # Configuration
//!
//! Set the following environment variables:
//!
//! - `PORT`            - Server port (default: 4020)
//! - `HOST`            - Bind address (default: 0.0.0.0)
//! - `MIDEN_RPC_URL`   - Miden node RPC URL (default: https://rpc.testnet.miden.io)
//! - `MIDEN_NETWORK`   - Network: "testnet" or "mainnet" (default: testnet)

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use axum::{Json, Router};
use std::env;
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use x402_chain_miden::chain::{MidenChainConfig, MidenChainProvider, MidenChainReference};
use x402_chain_miden::v2_miden_exact::facilitator::V2MidenExactFacilitator;
use x402_types::chain::ChainProviderOps;
use x402_types::proto;
use x402_types::scheme::X402SchemeFacilitator;

/// Shared application state.
struct AppState {
    facilitator: V2MidenExactFacilitator,
    faucet_id: String,
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
    });

    // Build router
    let app = Router::new()
        .route("/", get(root_handler))
        .route("/health", get(health_handler))
        .route("/supported", get(supported_handler))
        .route("/verify", post(verify_handler))
        .route("/settle", post(settle_handler))
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

    axum::serve(listener, app).await?;

    Ok(())
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
        Ok(response) => {
            let mut value = serde_json::to_value(response).unwrap();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("faucetId".to_string(), serde_json::json!(state.faucet_id));
            }
            (StatusCode::OK, Json(value))
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(serde_json::json!({ "error": e.to_string() })),
        ),
    }
}

async fn supported_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    match state.facilitator.supported().await {
        Ok(response) => (StatusCode::OK, Json(serde_json::to_value(response).unwrap())),
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
    let request = match serde_json::from_value::<proto::VerifyRequest>(body) {
        Ok(req) => req,
        Err(e) => {
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
        Ok(response) => (StatusCode::OK, Json(serde_json::to_value(response).unwrap())),
        Err(e) => {
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
    let request = match serde_json::from_value::<proto::SettleRequest>(body) {
        Ok(req) => req,
        Err(e) => {
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
        Ok(response) => (StatusCode::OK, Json(serde_json::to_value(response).unwrap())),
        Err(e) => {
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
