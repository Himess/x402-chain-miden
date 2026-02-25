# x402-chain-miden Architecture

## Overview

`x402-chain-miden` brings x402 HTTP payment protocol support to the Miden ZK rollup. This document describes the architecture and how the pieces fit together.

## System Architecture

```
┌─────────────────────────────────────────────────────────────────────┐
│                           x402 Payment Flow                         │
│                                                                     │
│  ┌──────────┐        ┌──────────────┐        ┌─────────────────┐   │
│  │  Client   │◀──────▶│   Resource   │◀──────▶│   Facilitator   │   │
│  │  (Agent)  │        │   Server     │        │   (Verifier)    │   │
│  └────┬─────┘        └──────────────┘        └───────┬─────────┘   │
│       │                                              │             │
│       │  P2ID Note + STARK Proof                     │  Submit     │
│       │  (client-side execution)                     │  ProvenTx   │
│       │                                              │             │
│       └──────────────────────────────────────────────┼─────────────│
│                                                      ▼             │
│                                              ┌───────────────┐     │
│                                              │  Miden Network │     │
│                                              │  (ZK Rollup)   │     │
│                                              └───────────────┘     │
└─────────────────────────────────────────────────────────────────────┘
```

## Crate Structure

```
x402-chain-miden/           (workspace root + chain crate)
├── src/
│   ├── lib.rs              Root module with feature-gated exports
│   ├── networks.rs         Known Miden networks (testnet/mainnet) + token deployments
│   ├── chain/
│   │   ├── mod.rs          Module re-exports
│   │   ├── types.rs        MidenAccountAddress, MidenChainReference, MidenTokenDeployment
│   │   ├── config.rs       MidenChainConfig for provider initialization
│   │   └── provider.rs     MidenChainProvider — Miden node RPC interface
│   └── v2_miden_exact/
│       ├── mod.rs          V2MidenExact struct + X402SchemeId impl
│       ├── types.rs        MidenExactPayload, type aliases, error types
│       ├── server.rs       Price tag generation (V2MidenExact::price_tag)
│       ├── client.rs       V2MidenExactClient — payment signing via MidenSignerLike
│       └── facilitator.rs  V2MidenExactFacilitator — verify + settle
│
├── facilitator/            Standalone facilitator HTTP server
│   ├── src/main.rs         Axum server with /verify, /settle, /supported
│   └── Dockerfile          Container deployment
│
├── examples/
│   ├── server-example/     Axum server with Miden payment wall
│   └── client-example/     Client making payments with mock signer
│
└── docs/
    └── architecture.md     This document
```

## Payment Flow (Detailed)

### 1. Server Creates Price Tag

The resource server specifies what payment is required:

```rust
use x402_chain_miden::V2MidenExact;
use x402_chain_miden::chain::MidenTokenDeployment;

let usdc = MidenTokenDeployment::testnet_usdc();
let price_tag = V2MidenExact::price_tag(
    recipient_account_id,
    usdc.amount(1_000_000),  // 1 USDC
);
```

This produces a `v2::PriceTag` containing:
- `scheme: "exact"`
- `network: "miden:testnet"` (CAIP-2)
- `amount: "1000000"`
- `asset: "<faucet_id>"` (USDC faucet on Miden)
- `pay_to: "<recipient_account_id>"`

### 2. Client Receives 402 and Signs Payment

When the client gets a `402 Payment Required`, `V2MidenExactClient::accept()` filters for Miden-compatible requirements and creates `PaymentCandidate`s.

Each candidate wraps a `MidenPayloadSigner` that:
1. Calls `MidenSignerLike::create_and_prove_p2id()` with recipient, faucet, amount
2. The signer (backed by `miden-client`) creates a P2ID note, executes locally, generates STARK proof
3. Wraps the result in `MidenExactPayload { from, proven_transaction, transaction_id }`
4. Serializes to JSON, base64-encodes, returns as `PAYMENT-SIGNATURE` header

### 3. Facilitator Verifies Payment

The facilitator receives the payment payload and:
1. Parses `VerifyRequest` from the raw JSON
2. Checks `accepted` requirements match `provided` requirements
3. **TODO**: Deserializes ProvenTransaction, verifies STARK proof
4. **TODO**: Checks output notes contain P2ID to correct recipient with correct amount
5. Returns `VerifyResponse::Valid { payer }`

### 4. Facilitator Settles Payment

After verification:
1. Hex-decodes the `proven_transaction` bytes
2. Submits to Miden node via `MidenChainProvider::submit_proven_transaction()`
3. Returns `SettleResponse::Success { payer, transaction, network }`

## EVM vs Miden: Key Differences

| Aspect | EVM (x402-chain-eip155) | Miden (x402-chain-miden) |
|--------|------------------------|--------------------------|
| **Payment mechanism** | ERC-3009 `transferWithAuthorization` | P2ID note creation |
| **Signing** | ECDSA / EIP-712 typed data | RPO-Falcon512 / ECDSA-K256 |
| **Execution** | On-chain (validators execute) | Client-side (ZK proven) |
| **Proof** | Tx simulation (eth_call) | STARK proof verification |
| **Privacy** | Public by default | Private by default |
| **Token identity** | ERC-20 contract address | Faucet AccountId |
| **Account ID** | 20-byte Ethereum address | 120-bit Miden AccountId |
| **Settlement** | Facilitator submits meta-tx | Facilitator submits ProvenTx |
| **Fee model** | Facilitator pays gas | Client proves, facilitator relays |
| **Chain ID** | `eip155:<chainId>` | `miden:testnet` / `miden:mainnet` |

## Feature Flags

| Feature | Description | Enables |
|---------|-------------|---------|
| `server` | Price tag generation | `V2MidenExact::price_tag()` |
| `client` | Payment signing | `V2MidenExactClient`, `MidenSignerLike` |
| `facilitator` | Verify + settle | `V2MidenExactFacilitator`, `MidenChainProvider` |
| `full` | All of the above | Everything |

## Integration with x402-rs Ecosystem

### As a Workspace Member

To use x402-chain-miden in the x402-rs workspace:

```toml
# x402-rs/Cargo.toml
[workspace]
members = [
    "crates/chains/x402-chain-miden",
    # ...
]
```

### With x402-axum (Server Middleware)

```rust
use x402_axum::X402Middleware;
use x402_chain_miden::V2MidenExact;

let x402 = X402Middleware::try_from("http://localhost:4020")?;
let app = Router::new()
    .route("/paid", get(handler).layer(
        x402.with_price_tag(V2MidenExact::price_tag(pay_to, amount))
    ));
```

### With x402-reqwest (Client Middleware)

```rust
use x402_reqwest::{ReqwestWithPayments, X402Client};
use x402_chain_miden::V2MidenExactClient;

let x402_client = X402Client::new()
    .register(V2MidenExactClient::new(miden_signer));

let client = Client::new()
    .with_payments(x402_client)
    .build();

// Automatically handles 402 → sign → retry
let res = client.get("https://api.example.com/data").send().await?;
```

### With x402-facilitator-local (Scheme Registry)

```rust
use x402_chain_miden::V2MidenExact;
use x402_types::scheme::SchemeBlueprints;

let schemes = SchemeBlueprints::new()
    .and_register(V2MidenExact);  // Register Miden scheme

let registry = SchemeRegistry::build(chains, schemes, &config.schemes);
let facilitator = FacilitatorLocal::new(registry);
```

## Deployment

### Facilitator Binary

```bash
# Run locally
cargo run -p x402-miden-facilitator

# With environment variables
PORT=4020 MIDEN_NETWORK=testnet MIDEN_RPC_URL=https://rpc.testnet.miden.io \
    cargo run -p x402-miden-facilitator

# Docker
docker build -t x402-miden-facilitator -f facilitator/Dockerfile .
docker run -p 4020:4020 -e MIDEN_NETWORK=testnet x402-miden-facilitator
```

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `PORT` | `4020` | Server port |
| `HOST` | `0.0.0.0` | Bind address |
| `MIDEN_RPC_URL` | `https://rpc.testnet.miden.io` | Miden node RPC URL |
| `MIDEN_NETWORK` | `testnet` | Network: testnet or mainnet |

## Future Work

1. **Wire in miden-client/miden-tx** — Real STARK proof verification and RPC submission
2. **Testnet faucet IDs** — Update placeholder faucet IDs when Miden deploys standard tokens
3. **Privacy mode** — Support private notes for confidential payments
4. **Agent SDK** — TypeScript + Rust SDK for building payment-capable AI agents
5. **MidenAgentKit CLI** — `npx create-miden-agent` scaffolding tool
