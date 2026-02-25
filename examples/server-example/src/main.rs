//! Example Axum server with Miden payment wall.
//!
//! This example demonstrates how a resource server would create payment
//! requirements using x402-chain-miden. In a production setup, this would
//! be used with the `x402-axum` middleware crate.
//!
//! # Running
//!
//! ```bash
//! cargo run -p x402-miden-server-example
//! ```
//!
//! # Endpoints
//!
//! - `GET /`              - Free endpoint
//! - `GET /paid-content`  - Returns 402 with Miden payment requirements
//! - `GET /price-info`    - Shows the price tag configuration

use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use x402_chain_miden::chain::MidenTokenDeployment;
use x402_chain_miden::V2MidenExact;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let app = Router::new()
        .route("/", get(root_handler))
        .route("/paid-content", get(paid_content_handler))
        .route("/price-info", get(price_info_handler));

    let port = std::env::var("PORT").unwrap_or_else(|_| "3000".to_string());
    let bind = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    tracing::info!("Server listening on {bind}");

    axum::serve(listener, app).await?;
    Ok(())
}

/// Free endpoint - no payment required.
async fn root_handler() -> impl IntoResponse {
    Json(serde_json::json!({
        "message": "Welcome! Access /paid-content for premium data.",
        "chain": "miden",
    }))
}

/// Paid endpoint - returns 402 with Miden payment requirements.
///
/// In production, the `x402-axum` middleware would handle this automatically.
/// This example manually constructs the 402 response to show the structure.
async fn paid_content_handler() -> impl IntoResponse {
    // Create the price tag for 1 USDC on Miden testnet
    let recipient = "0xaabbccddee11223344556677889900aabbccddee11223344556677889900"
        .parse()
        .expect("valid recipient address");

    let usdc = MidenTokenDeployment::testnet_usdc();
    let price_tag = V2MidenExact::price_tag(recipient, usdc.amount(1_000_000)); // 1 USDC

    // Build the PaymentRequired response
    // In production, x402-axum middleware does this automatically
    let payment_required = serde_json::json!({
        "x402Version": 2,
        "error": "Payment required to access this resource",
        "resource": {
            "url": "/paid-content",
            "description": "Premium market data feed",
            "mimeType": "application/json",
        },
        "accepts": [{
            "scheme": price_tag.requirements.scheme,
            "network": price_tag.requirements.network.to_string(),
            "amount": price_tag.requirements.amount,
            "payTo": price_tag.requirements.pay_to.to_string(),
            "asset": price_tag.requirements.asset.to_string(),
            "maxTimeoutSeconds": price_tag.requirements.max_timeout_seconds,
        }],
    });

    // Encode as base64 for the PAYMENT-REQUIRED header
    let encoded = base64_encode(&serde_json::to_vec(&payment_required).unwrap());

    (
        StatusCode::PAYMENT_REQUIRED,
        [("PAYMENT-REQUIRED", encoded)],
        Json(payment_required),
    )
}

/// Shows the price tag configuration without requiring payment.
async fn price_info_handler() -> impl IntoResponse {
    let recipient = "0xaabbccddee11223344556677889900aabbccddee11223344556677889900"
        .parse()
        .expect("valid recipient address");

    let usdc = MidenTokenDeployment::testnet_usdc();
    let price_tag = V2MidenExact::price_tag(recipient, usdc.amount(1_000_000));

    Json(serde_json::json!({
        "scheme": "exact",
        "network": price_tag.requirements.network.to_string(),
        "amount": price_tag.requirements.amount,
        "asset": price_tag.requirements.asset.to_string(),
        "pay_to": price_tag.requirements.pay_to.to_string(),
        "max_timeout_seconds": price_tag.requirements.max_timeout_seconds,
        "description": "1 USDC on Miden testnet",
    }))
}

fn base64_encode(input: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::with_capacity((input.len() + 2) / 3 * 4);
    let mut i = 0;
    while i + 2 < input.len() {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8) | input[i + 2] as u32;
        out.extend_from_slice(&[
            CHARS[(n >> 18 & 0x3F) as usize],
            CHARS[(n >> 12 & 0x3F) as usize],
            CHARS[(n >> 6 & 0x3F) as usize],
            CHARS[(n & 0x3F) as usize],
        ]);
        i += 3;
    }
    let rem = input.len() - i;
    if rem == 2 {
        let n = ((input[i] as u32) << 16) | ((input[i + 1] as u32) << 8);
        out.extend_from_slice(&[
            CHARS[(n >> 18 & 0x3F) as usize],
            CHARS[(n >> 12 & 0x3F) as usize],
            CHARS[(n >> 6 & 0x3F) as usize],
            b'=',
        ]);
    } else if rem == 1 {
        let n = (input[i] as u32) << 16;
        out.extend_from_slice(&[
            CHARS[(n >> 18 & 0x3F) as usize],
            CHARS[(n >> 12 & 0x3F) as usize],
            b'=',
            b'=',
        ]);
    }
    String::from_utf8(out).unwrap()
}
