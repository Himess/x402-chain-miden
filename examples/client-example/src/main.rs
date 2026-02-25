//! Example client that pays for HTTP requests using Miden.
//!
//! This example demonstrates how a client (human or AI agent) would:
//! 1. Make a request to a protected endpoint
//! 2. Receive a 402 Payment Required response
//! 3. Create a P2ID payment on Miden
//! 4. Retry the request with the payment signature
//!
//! In production, `x402-reqwest` middleware handles steps 2-4 automatically.
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

use async_trait::async_trait;
use serde::Deserialize;
use x402_chain_miden::v2_miden_exact::client::MidenSignerLike;
use x402_chain_miden::V2MidenExactClient;
use x402_types::scheme::client::X402Error;

/// A mock Miden signer for demonstration purposes.
///
/// In production, this would wrap `miden-client`'s wallet and transaction
/// proving capabilities. The signer creates P2ID notes, executes transactions
/// in Miden VM, generates STARK proofs, and returns the serialized
/// ProvenTransaction.
#[derive(Debug, Clone)]
struct MockMidenSigner {
    account_id: String,
}

#[async_trait]
impl MidenSignerLike for MockMidenSigner {
    fn account_id(&self) -> String {
        self.account_id.clone()
    }

    async fn create_and_prove_p2id(
        &self,
        recipient: &str,
        faucet_id: &str,
        amount: u64,
    ) -> Result<(String, String), X402Error> {
        // In production, this would:
        // 1. Create a P2ID note: sender → recipient, amount of faucet token
        // 2. Build a TransactionRequest with the note as output
        // 3. Execute the transaction in Miden VM (client-side)
        // 4. Generate a STARK proof using LocalTransactionProver
        // 5. Serialize the ProvenTransaction to hex
        // 6. Return (proven_tx_hex, transaction_id_hex)
        //
        // Example with real miden-client:
        // ```
        // let note = P2idNote::create(sender, recipient, assets, NoteType::Public, ...)?;
        // let tx_request = TransactionRequest::new().with_output_notes(vec![note]);
        // let executed_tx = executor.execute_transaction(account_id, block, inputs, tx_request)?;
        // let proven_tx = prover.prove(executed_tx)?;
        // let tx_bytes = proven_tx.to_bytes();
        // let tx_id = proven_tx.id().to_hex();
        // ```

        tracing::info!(
            recipient = recipient,
            faucet_id = faucet_id,
            amount = amount,
            "Creating mock P2ID payment"
        );

        // Mock: return placeholder hex values
        let mock_proven_tx = format!(
            "deadbeef{:0>16x}{:0>16x}{}",
            amount,
            self.account_id.len(),
            hex::encode(recipient.as_bytes())
        );
        let mock_tx_id = format!("{:0>64x}", amount);

        Ok((mock_proven_tx, mock_tx_id))
    }
}

#[derive(Debug, Deserialize)]
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

    // Create a mock Miden signer
    // In production: wrap a real miden-client wallet
    let signer = MockMidenSigner {
        account_id: "0x00112233445566778899aabbccddeeff00112233445566778899aabbccddee".to_string(),
    };

    // Create the x402 Miden client
    let _client = V2MidenExactClient::new(signer.clone());

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
                network = accept.get("network").and_then(|v| v.as_str()).unwrap_or("?"),
                amount = accept.get("amount").and_then(|v| v.as_str()).unwrap_or("?"),
                "Payment option"
            );
        }

        // Step 3: Create payment using the signer
        // In production, x402-reqwest middleware does this automatically:
        //   let x402_client = X402Client::new()
        //       .register(V2MidenExactClient::new(real_signer));
        //   let client = Client::new().with_payments(x402_client).build();
        //   let res = client.get(endpoint).send().await?;
        //
        // For this demo, we show the manual flow:

        if let Some(accept) = payment_required.accepts.first() {
            let recipient = accept
                .get("payTo")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let faucet = accept
                .get("asset")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let amount: u64 = accept
                .get("amount")
                .and_then(|v| v.as_str())
                .and_then(|s| s.parse().ok())
                .unwrap_or(0);

            let (proven_tx, tx_id) = signer
                .create_and_prove_p2id(recipient, faucet, amount)
                .await?;

            tracing::info!(
                tx_id = tx_id,
                proven_tx_len = proven_tx.len(),
                "Payment created — would retry with PAYMENT-SIGNATURE header"
            );

            // Step 4: In production, retry with payment header
            // let payment_payload = base64(json({
            //     "x402Version": 2,
            //     "accepted": accept,
            //     "payload": { "from": signer.account_id(), "provenTransaction": proven_tx, "transactionId": tx_id },
            // }));
            // let response = http.get(&endpoint)
            //     .header("PAYMENT-SIGNATURE", payment_payload)
            //     .send().await?;
        }
    } else {
        let body = response.text().await?;
        tracing::info!("Response: {body}");
    }

    Ok(())
}
