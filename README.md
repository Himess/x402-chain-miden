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

## Status

This crate provides the complete type system and trait implementations for x402-Miden integration. The provider module (`MidenChainProvider`) contains TODO stubs for:

- Submitting proven transactions to the Miden node RPC
- Querying account balances from account vaults
- Full STARK proof verification in the facilitator

These will be implemented once the `miden-client` and `miden-tx` crates are added as dependencies.

## License

Apache-2.0
