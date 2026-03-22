//! Example client that pays for HTTP requests using Miden.
//!
//! This example demonstrates how a client (human or AI agent) would use
//! the lightweight payment flow (bobbinth's design):
//!
//! 1. Make a request to a protected endpoint
//! 2. Receive a 402 Payment Required response with a lightweight requirement
//! 3. Create a P2ID payment, submit to Miden network
//! 4. Send back the lightweight payment header (note_id + inclusion proof)
//!
//! # Running
//!
//! ```bash
//! # Start the server example first:
//! cargo run -p x402-miden-server-example
//!
//! # Then run this client:
//! ENDPOINT=http://localhost:3000/paid-content cargo run -p x402-miden-client-example
//! ```

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PaymentRequired {
    x402_version: u8,
    error: Option<String>,
    accepts: Vec<serde_json::Value>,
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let endpoint = std::env::var("ENDPOINT")
        .unwrap_or_else(|_| "http://localhost:3000/paid-content".to_string());

    // Step 1: Make initial request
    tracing::info!("Requesting {endpoint}");
    let http = reqwest::Client::new();
    let response = http.get(&endpoint).send().await?;

    tracing::info!("Status: {}", response.status());

    if response.status() == reqwest::StatusCode::PAYMENT_REQUIRED {
        // Step 2: Parse payment requirements
        let payment_required: PaymentRequired = response.json().await?;
        tracing::info!(
            version = payment_required.x402_version,
            error = ?payment_required.error,
            accepts_count = payment_required.accepts.len(),
            "Received 402 Payment Required"
        );

        for (i, accept) in payment_required.accepts.iter().enumerate() {
            tracing::info!(
                index = i,
                scheme = accept.get("scheme").and_then(|v| v.as_str()).unwrap_or("?"),
                network = accept
                    .get("network")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?"),
                amount = accept.get("amount").and_then(|v| v.as_str()).unwrap_or("?"),
                "Payment option"
            );
        }

        // In production with the lightweight flow:
        // 1. Parse the LightweightPaymentRequirement from the 402 response
        // 2. Create a P2ID note using the recipient_digest
        // 3. Prove and submit the transaction to the Miden network
        // 4. Sync state to get the note inclusion proof
        // 5. Send {note_id, block_num, inclusion_proof} to the server
        //
        // Example with LightweightMidenPayer:
        //   let payer = LightweightMidenPayer::new(account_id, client);
        //   let header = payer.create_and_submit_payment(&requirement).await?;
        //   // Send header to server's /verify-lightweight endpoint
        tracing::info!(
            "In production, the agent would create and submit a P2ID note, \
             then send a lightweight payment header to the server."
        );
    } else {
        let body = response.text().await?;
        tracing::info!("Response: {body}");
    }

    Ok(())
}
