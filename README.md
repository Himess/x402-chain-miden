# x402-chain-miden

Miden blockchain support for the [x402 payment protocol](https://x402.org).

This crate provides a V2 "exact" payment scheme implementation for the [Miden](https://miden.xyz) ZK rollup, enabling HTTP 402 Payment Required flows with P2ID (Pay-to-ID) note-based payments.

## Architecture

Unlike EVM chains that use `transferWithAuthorization` (ERC-3009) for gasless token transfers, Miden uses a note-based UTXO model with client-side proving:

| EVM (eip155) | Miden |
|---|---|
| ERC-20 token address | Faucet AccountId |
| `transferWithAuthorization` | P2ID note creation |
| ECDSA/EIP-712 signing | RPO-Falcon512 / ECDSA-K256 |
| On-chain execution | Client-side execution + STARK proof |
| Transaction hash | TransactionId |
| EOA address (0x...) | AccountId (120-bit, hex/bech32) |
| Chain ID (uint64) | CAIP-2 `miden:testnet` / `miden:mainnet` |

### Payment Flow

```
Client                    Server                    Facilitator
  |                         |                           |
  |-- GET /resource ------->|                           |
  |<-- 402 + PriceTag ------|                           |
  |                         |                           |
  | [Create P2ID note]      |                           |
  | [Execute in Miden VM]   |                           |
  | [Generate STARK proof]  |                           |
  |                         |                           |
  |-- GET /resource ------->|                           |
  |   + Payment-Signature   |-- verify(payload) ------->|
  |                         |<-- VerifyResponse --------|
  |<-- 200 + resource ------|                           |
  |                         |-- settle(payload) ------->|
  |                         |   [Submit ProvenTx]       |
  |                         |<-- SettleResponse --------|
```

## Feature Flags

| Feature | Description |
|---|---|
| `server` | Server-side price tag generation |
| `client` | Client-side payment signing (P2ID + proving) |
| `facilitator` | Facilitator-side verification and settlement |
| `full` | All of the above |

## Usage

### Server: Creating a Price Tag

```rust,ignore
use x402_chain_miden::V2MidenExact;
use x402_chain_miden::chain::MidenTokenDeployment;

let usdc = MidenTokenDeployment::testnet_usdc();
let price_tag = V2MidenExact::price_tag(
    "0x1234abcd...".parse().unwrap(),
    usdc.amount(1_000_000), // 1 USDC (6 decimals)
);
```

### Client: Signing a Payment

```rust,ignore
use x402_chain_miden::V2MidenExactClient;
use x402_chain_miden::v2_miden_exact::client::MidenSignerLike;

let client = V2MidenExactClient::new(miden_signer);
let candidates = client.accept(&payment_required);
```

### Facilitator: Verifying and Settling

```rust,ignore
use x402_chain_miden::V2MidenExact;
use x402_chain_miden::chain::{MidenChainConfig, MidenChainProvider, MidenChainReference};
use x402_types::scheme::X402SchemeFacilitatorBuilder;

let config = MidenChainConfig {
    chain_reference: MidenChainReference::testnet(),
    rpc_url: "https://rpc.testnet.miden.io".to_string(),
};
let provider = MidenChainProvider::from_config(&config);
let facilitator = V2MidenExact.build(provider, None).unwrap();

let verify_response = facilitator.verify(&verify_request).await?;
let settle_response = facilitator.settle(&settle_request).await?;
```

## CAIP-2 Chain Identifiers

| Network | Chain ID |
|---|---|
| Miden Testnet | `miden:testnet` |
| Miden Mainnet | `miden:mainnet` |

## Scheme Identifier

- **Namespace:** `miden`
- **Scheme:** `exact`
- **ID:** `v2-miden-exact`

## Workspace Structure

```
x402-chain-miden/
├── src/                        # Core library (x402-chain-miden crate)
│   ├── lib.rs                  # Public API re-exports
│   ├── chain/                  # Chain types, provider, config
│   ├── networks.rs             # Known networks + USDC deployments
│   └── v2_miden_exact/         # V2 exact scheme (server, client, facilitator)
├── facilitator/                # Standalone facilitator HTTP server
│   ├── src/main.rs             # Axum server: /verify, /settle, /supported, /health
│   └── Dockerfile              # Multi-stage Docker build
├── examples/
│   ├── server-example/         # Resource server with 402 payment wall
│   └── client-example/         # Client with mock Miden signer
├── tests/
│   └── integration_test.rs     # 34 integration tests + 7 miden-native
└── docs/
    └── architecture.md         # Full architecture documentation
```

## Running

### Facilitator Server

```bash
# Default: testnet, port 4020
cargo run -p x402-miden-facilitator

# Custom config
MIDEN_NETWORK=mainnet MIDEN_RPC_URL=https://rpc.mainnet.miden.io PORT=8080 \
  cargo run -p x402-miden-facilitator

# Docker
docker build -t x402-miden-facilitator -f facilitator/Dockerfile .
docker run -p 4020:4020 x402-miden-facilitator
```

### Examples

```bash
# Server example (port 3000)
cargo run -p x402-miden-server-example

# Client example
cargo run -p x402-miden-client-example
```

### Tests

```bash
# All tests (65 total: 22 unit + 34 integration + 7 miden-native + 2 doc-tests)
cargo test --workspace --features full
```

## License

Apache-2.0
