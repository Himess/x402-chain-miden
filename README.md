# x402-chain-miden

Miden blockchain support for the [x402 payment protocol](https://x402.org).

This crate implements HTTP 402 Payment Required flows on the [Miden](https://miden.xyz) ZK rollup using a lightweight verification design based on note inclusion proofs ([0xMiden/node#1796](https://github.com/0xMiden/node/issues/1796)).

## Architecture

Instead of sending a full `ProvenTransaction` (~100 KB) to the facilitator, the agent submits the transaction directly to the Miden network and sends only a compact note inclusion proof (~200 bytes) to the server.

| EVM (eip155) | Miden |
|---|---|
| ERC-20 token address | Faucet AccountId |
| `transferWithAuthorization` | P2ID note creation |
| On-chain execution | Client-side execution + STARK proof |
| Transaction hash | NoteId + inclusion proof |
| EOA address (0x...) | AccountId (hex) |
| Chain ID (uint64) | CAIP-2 `miden:testnet` / `miden:mainnet` |

### Payment Flow

```
Agent                         Server / Facilitator
  |                                   |
  |-- GET /resource ----------------->|
  |<-- 402 {recipient_digest, asset,  |
  |         note_tag, serial_num} ----|
  |                                   |
  | Create P2ID note                  |
  | STARK prove + submit to network   |
  | sync_state() -> inclusion proof   |
  |                                   |
  |-- {note_id, block_num,           |
  |    inclusion_proof} ------------>|
  |                                   |
  |    Verify: NoteId matches?        |
  |    SparseMerklePath.verify()      |
  |                                   |
  |<-- 200 OK ------------------------|
```

The server generates a `recipient_digest` from a random `serial_num`, the P2ID script root, and the recipient's account ID. The agent constructs a matching P2ID note, submits it to the network, then sends back the note ID and Merkle inclusion proof. The server verifies the NoteId matches and the proof is valid against the block's note tree.

## Feature Flags

| Feature | Description |
|---|---|
| `server` | Server-side payment requirement generation |
| `client` | Client-side lightweight payment creation |
| `facilitator` | Facilitator-side chain state and lightweight verification |
| `miden-native` | Real RPO256 digest computation via `miden-protocol` |
| `miden-client-native` | Full `miden-client` integration (RPC, proving, submission) |
| `full` | Enables `server` + `client` + `facilitator` |

## Usage

### Server: Creating a Payment Requirement

```rust,ignore
use x402_chain_miden::lightweight::server::create_payment_requirement;
use x402_types::chain::ChainId;

let (requirement, context) = create_payment_requirement(
    "0xaabbccddeeff0011",    // pay_to (recipient account ID)
    "0x37d5977a8e16d820",    // asset faucet ID
    1_000_000,               // amount (1 USDC, 6 decimals)
    12345,                   // note_tag
    ChainId::new("miden", "testnet"),
)?;
// Send `requirement` in the HTTP 402 response body.
// Store `context` server-side for later verification.
```

### Client: Submitting a Payment

```rust,ignore
use x402_chain_miden::lightweight::LightweightMidenPayer;

// LightweightMidenPayer wraps a miden_client::Client
let payer = LightweightMidenPayer::new(account_id, client);
let header = payer.create_and_submit_payment(&requirement).await?;
// Send `header` (note_id + block_num + inclusion_proof) to the server.
```

### Facilitator: Verifying a Payment

```rust,ignore
use x402_chain_miden::lightweight::{
    FacilitatorChainState, verify_lightweight_payment_full,
};
use x402_chain_miden::chain::MidenChainReference;

let chain_state = FacilitatorChainState::new(
    "https://rpc.testnet.miden.io".to_string(),
    MidenChainReference::testnet(),
);

let result = verify_lightweight_payment_full(&context, &header, &chain_state).await?;
assert!(result.valid);
```

## CAIP-2 Chain Identifiers

| Network | Chain ID |
|---|---|
| Miden Testnet | `miden:testnet` |
| Miden Mainnet | `miden:mainnet` |

## Workspace Structure

```
x402-chain-miden/
├── src/                        # Core library (x402-chain-miden crate)
│   ├── lib.rs                  # Public API re-exports
│   ├── chain/                  # Chain types, provider, config
│   ├── lightweight/            # Lightweight verification (bobbinth's design)
│   │   ├── server.rs           # 402 response generation, create_payment_requirement
│   │   ├── client.rs           # Agent-side payment creation (LightweightMidenPayer)
│   │   ├── verification.rs     # NoteId reconstruction + SparseMerklePath verification
│   │   ├── chain_state.rs      # FacilitatorChainState (block header cache)
│   │   └── types.rs            # Wire-format types
│   ├── v2_miden_exact/         # V2 exact scheme (price tags, x402-types integration)
│   └── networks.rs             # Known networks + token deployments
├── facilitator/                # Standalone facilitator HTTP server (Axum)
│   ├── src/main.rs             # /payment-requirement, /verify-lightweight, /health
│   └── Dockerfile              # Multi-stage Docker build
├── examples/
│   ├── server-example/         # Resource server with 402 payment wall
│   └── client-example/         # Client demonstrating the lightweight flow
└── tests/
    ├── integration_test.rs     # Integration tests
    ├── miden_native_test.rs    # Tests requiring miden-native feature
    └── e2e_testnet.rs          # End-to-end testnet tests
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
cargo test --workspace --features full
```

## License

Apache-2.0
