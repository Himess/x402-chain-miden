# Privacy Architecture for x402-chain-miden

## Executive Summary

**Recommended approach: Phased strategy leveraging Miden's native privacy primitives.**

x402-chain-miden currently requires `NoteType::Public` so the facilitator can verify payment details (recipient, amount, faucet) in the `ProvenTransaction`. This exposes all payment data on-chain, nullifying Miden's privacy advantage over EVM chains.

The solution is **not** to build custom ZK circuits. Miden already provides the cryptographic primitives we need:

- **Phase 1 (current)**: `NoteType::Public` — everything works, zero privacy.
- **Phase 2 (recommended near-term)**: **Trusted Facilitator with `NoteType::Private`** — client shares full note data with the facilitator off-chain via the x402 payload. Facilitator verifies the note binds to the `ProvenTransaction` via `NoteId` commitment, then submits. On-chain: only the note hash. Privacy from everyone except the facilitator.
- **Phase 3 (future)**: **`NoteType::Encrypted` + facilitator verification** — note data encrypted to recipient's `SealingKey` and attached on-chain. Recipient discovers and decrypts autonomously. Facilitator still receives note data off-chain for verification, but on-chain observers see only ciphertext.

**Phase 2 is implementable today** with ~100 lines of code changes. It provides Web2-equivalent privacy (the facilitator is analogous to a payment processor — it sees the transaction, but the public doesn't). This aligns with Miden's current "privacy training wheels" phase where the operator already sees transaction data.

---

## Table of Contents

1. [The Problem](#1-the-problem)
2. [Miden's Privacy Primitives](#2-midens-privacy-primitives)
3. [How Existing Projects Solve This](#3-how-existing-projects-solve-this)
4. [Approach Evaluation](#4-approach-evaluation)
5. [Recommended Architecture: Phase 2](#5-recommended-architecture-phase-2)
6. [Future: Phase 3 with Encrypted Notes](#6-future-phase-3-with-encrypted-notes)
7. [Alternative Approaches (Evaluated and Deferred)](#7-alternative-approaches-evaluated-and-deferred)
8. [Implementation Roadmap](#8-implementation-roadmap)
9. [Sources](#9-sources)

---

## 1. The Problem

The x402 payment flow requires a **facilitator** to verify that a client's payment meets the server's requirements before granting access. On EVM chains, this is done by inspecting a signed transaction. On Miden, the facilitator inspects the `ProvenTransaction`'s output notes.

Currently:
```
Client → creates P2ID note (NoteType::Public)
       → proves transaction (STARK proof)
       → sends ProvenTransaction to facilitator

Facilitator → deserializes ProvenTransaction
            → finds OutputNote::Full (because Public)
            → verifies: P2ID script root, recipient, faucet, amount
            → verifies STARK proof
            → submits to Miden node
```

The problem: `OutputNote::Full` for public notes exposes **everything** on-chain — sender, recipient, amount, asset type. Anyone observing the Miden note database can see exactly who paid whom and how much.

### What we want

The facilitator must be able to verify payment details, but **the chain** should not reveal them. This is the core tension: verification requires data visibility, but privacy requires data hiding.

---

## 2. Miden's Privacy Primitives

### 2.1 NoteType Enum (Three Variants)

```rust
// miden-protocol v0.13.3 — src/note/note_type.rs
pub enum NoteType {
    Public    = 0b01,  // Full note data on-chain
    Private   = 0b10,  // Only note hash on-chain
    Encrypted = 0b11,  // Encrypted note data on-chain
}
```

**Critical behavior in `OutputNote::shrink()`** (called during proving):

| NoteType    | Before proving          | After proving (in ProvenTransaction) |
|-------------|-------------------------|--------------------------------------|
| `Public`    | `OutputNote::Full`      | `OutputNote::Full` — all data visible |
| `Encrypted` | `OutputNote::Full`      | `OutputNote::Full` — encrypted payload preserved |
| `Private`   | `OutputNote::Full`      | `OutputNote::Header` — only NoteId + metadata |

```rust
// miden-protocol — src/transaction/outputs.rs
pub fn shrink(&self) -> Self {
    match self {
        OutputNote::Full(note) if note.metadata().is_private() => {
            OutputNote::Header(note.header().clone())  // Private → stripped
        },
        OutputNote::Partial(note) => OutputNote::Header(note.header().clone()),
        _ => self.clone(),  // Public and Encrypted → kept as Full
    }
}
```

Key insight: **`is_private()` returns true ONLY for `NoteType::Private`**. Encrypted notes pass through unchanged.

### 2.2 What's Visible On-Chain Per NoteType

| Data                 | Public | Encrypted | Private |
|----------------------|--------|-----------|---------|
| NoteId (commitment)  | Yes    | Yes       | Yes     |
| NoteMetadata (sender, tag, type) | Yes | Yes | Yes |
| NoteRecipient (script, inputs) | Yes | Encrypted | No |
| NoteAssets (amounts)  | Yes    | Encrypted | No      |
| Nullifier linkable?   | Yes    | Yes       | No      |

### 2.3 NoteId as Cryptographic Binding

The `NoteId` is derived from the full note content:

```
NoteId = hash(recipient_digest, asset_commitment)
```

Where:
```
recipient_digest = hash(hash(hash(serial_num, [0;4]), script_root), input_commitment)
asset_commitment = hash(assets)
```

This means: **if you know the full note, you can compute the NoteId. If the computed NoteId matches the one in the ProvenTransaction, the note is authentic.** This is the cryptographic binding that enables off-chain verification with on-chain privacy.

### 2.4 Note Encryption (SealingKey)

Miden v0.12+ supports address-level encryption keys:

```rust
// miden-protocol — src/address/routing_parameters.rs
pub struct RoutingParameters {
    interface: AddressInterface,
    note_tag_len: Option<u8>,
    encryption_key: Option<SealingKey>,  // recipient's public key
}

pub enum SealingKey {
    X25519XChaCha20Poly1305(PublicKey),
    K256XChaCha20Poly1305(PublicKey),
    X25519AeadRpo(PublicKey),
    K256AeadRpo(PublicKey),
}
```

The sender encrypts `NoteDetails` (assets + full recipient) with the recipient's `SealingKey` and stores the ciphertext in `NoteAttachment`. Only the holder of the corresponding `UnsealingKey` can decrypt.

### 2.5 NoteTag for Privacy-Preserving Discovery

```rust
pub struct NoteTag(u32);  // 32-bit best-effort filter
```

Tags encode a configurable number of bits (0–30) of the recipient's account ID. Fewer bits = more privacy (more false positives during sync), more bandwidth. Recipients configure their preferred `note_tag_len` in their `Address` routing parameters.

### 2.6 "Privacy Training Wheels" Phase

From [Miden's privacy blog](https://miden.xyz/resource/blog/privacy):

> "In the testnet and initial mainnet of Miden, clients will be required to send all transaction data, along with the transaction proof, to the operator. This interim measure, which we refer to internally as 'privacy training wheels,' already offers Web2-like privacy."

Users get privacy from each other, not from the node operator. This is relevant: our facilitator is analogous to the operator in the training wheels phase.

---

## 3. How Existing Projects Solve This

### 3.1 Spark (compolabs) — Dark Pool Order Book

Spark builds a privacy-preserving CLOB on Miden using:

- **SWAPp notes** with partial fills — order details hidden until execution
- **PAYBACK_RECIPIENT hash** — identity commitment in note inputs, verified inside the ZK execution without revealing the account ID on-chain
- **Obfuscator account pattern** — a proxy account that relays orders, decoupling user identity from on-chain activity
- **MAST (Merkelized Abstract Syntax Trees)** — only the executed branch of a note script is revealed during consumption

**Relevance to x402**: Spark's obfuscator pattern is analogous to our facilitator. They accept that the operator/obfuscator sees order details, but on-chain observers don't. This validates the "Trusted Facilitator" approach.

Sources:
- [compolabs/spark-miden-v1](https://github.com/compolabs/spark-miden-v1)
- [Polygon Blog: Composability Labs / Spark](https://polygon.technology/blog/miden-pioneers-composability-labs-is-building-spark-a-superfast-onchain-clob-with-a-state-minimized-approach)
- [Medium: Privacy-Focused CLOB on Miden](https://medium.com/sprkfi/developing-a-privacy-focused-decentralized-order-book-exchange-on-polygon-miden-95547119543f)

### 3.2 Private State Management (PSM) — Miden + OpenZeppelin

PSM is a pattern for private multi-party coordination:

- Participants share **state deltas** (not full state) via off-chain channels
- A coordinator verifies `TransactionSummary` commitments without seeing private state
- Designed for private multisigs but applicable to payment facilitation

**Relevance**: The PSM coordinator role maps directly to our facilitator role. The pattern validates sharing minimal data off-chain for verification.

Source: [Miden Blog: Multisigs, But Private](https://miden.xyz/resource/blog/private-multisig)

### 3.3 No Existing x402-Like Protocol on Miden

Extensive search found zero implementations of HTTP 402 payment verification on Miden. The PRXVT SDK implements x402 privacy but on EVM chains (Base/Polygon) using Groth16, not Miden. **We would be the first x402 implementation on Miden.**

---

## 4. Approach Evaluation

### Approach A: Trusted Facilitator + NoteType::Private

**How it works:**
1. Client creates P2ID with `NoteType::Private`
2. Before proving, extracts full note from `TransactionResult`
3. Proves transaction — note becomes `OutputNote::Header` in ProvenTransaction
4. Sends ProvenTransaction + full note data to facilitator (in x402 payload)
5. Facilitator: computes `NoteId` from full note, verifies it matches ProvenTransaction output, inspects payment details, verifies STARK proof, submits

**On-chain**: Only note hash. No payment details visible.

| Criterion | Rating |
|-----------|--------|
| Implementation difficulty | **Easy** — ~100 LOC changes |
| Privacy from chain observers | **Full** — only hash on-chain |
| Privacy from facilitator | **None** — facilitator sees everything |
| Recipient delivery | Off-chain (P2P transport, direct) |
| x402 flow compatibility | **Drop-in** — same payload structure, extra field |
| Miden ecosystem alignment | **Strong** — matches "training wheels" phase |

### Approach B: NoteType::Encrypted + Off-chain Facilitator Verification

**How it works:**
1. Client creates P2ID with `NoteType::Encrypted`
2. Encrypts note details with recipient's `SealingKey`, attaches as `NoteAttachment`
3. Proves transaction — `OutputNote::Full` preserved (encrypted payload)
4. Sends ProvenTransaction + unencrypted note data to facilitator
5. Facilitator verifies note binding + payment details + STARK proof, submits
6. Recipient discovers encrypted note on-chain, decrypts with `UnsealingKey`

**On-chain**: Encrypted blob. Only recipient can decrypt.

| Criterion | Rating |
|-----------|--------|
| Implementation difficulty | **Medium** — requires SealingKey integration, encryption API |
| Privacy from chain observers | **Full** — ciphertext only |
| Privacy from facilitator | **None** — facilitator still sees payment details |
| Recipient delivery | **Self-contained** — recipient decrypts from chain |
| x402 flow compatibility | Good — additional encryption step |
| Miden ecosystem alignment | **Excellent** — uses native Miden encryption |

### Approach C: ZK Proof of Payment (Secondary Proof)

**How it works:**
1. Client creates P2ID with `NoteType::Private`
2. Client generates an additional ZK proof attesting: "I created a note paying X tokens of faucet F to recipient R" — without revealing X, F, R directly
3. Facilitator verifies this secondary proof against public commitments

| Criterion | Rating |
|-----------|--------|
| Implementation difficulty | **Very Hard** — custom ZK circuit design |
| Privacy from chain observers | **Full** |
| Privacy from facilitator | **Full** — facilitator learns nothing beyond validity |
| Miden ecosystem alignment | Possible (Miden VM is a ZK prover) but no existing patterns |

### Approach D: Custom Note Script (On-chain Verification)

**How it works:**
1. Embed facilitator verification logic in the P2ID note script
2. Note can only be consumed if the facilitator's account has authorized it
3. No off-chain verification needed — Miden VM handles it

| Criterion | Rating |
|-----------|--------|
| Implementation difficulty | **Hard** — custom MASM note script |
| Privacy from chain observers | Depends on note type |
| x402 flow compatibility | **Poor** — fundamentally different model |

### Approach E: Selective Disclosure (Amount + Recipient Only)

**How it works:**
1. Client shares only the specific note fields (amount, recipient, faucet) needed for verification
2. Remaining fields (serial_num, full script) stay private
3. Facilitator recomputes partial commitment and verifies binding

| Criterion | Rating |
|-----------|--------|
| Implementation difficulty | **Medium** — but `NoteInputs` has no selective disclosure mechanism |
| Privacy gain | **Minimal** — amount and recipient are the sensitive parts |
| Miden support | **None** — NoteInputs is all-or-nothing (no Merkle proof path) |

---

## 5. Recommended Architecture: Phase 2

### Trusted Facilitator + NoteType::Private

This is the recommended near-term approach. It provides **full on-chain privacy** while requiring minimal code changes and aligning perfectly with Miden's current architecture.

### 5.1 Flow Diagram

```
CLIENT                           FACILITATOR                    MIDEN NETWORK
  │                                   │                              │
  │ 1. GET /resource                  │                              │
  │──────────────────────────────────>│                              │
  │   402 Payment Required            │                              │
  │<──────────────────────────────────│                              │
  │                                   │                              │
  │ 2. Create P2ID note               │                              │
  │    (NoteType::Private)            │                              │
  │                                   │                              │
  │ 3. Extract full note from         │                              │
  │    TransactionResult              │                              │
  │                                   │                              │
  │ 4. Prove transaction              │                              │
  │    (note shrunk to Header)        │                              │
  │                                   │                              │
  │ 5. Send x402 payload:             │                              │
  │    - ProvenTransaction            │                              │
  │    - TransactionInputs            │                              │
  │    - Full Note (off-chain)        │                              │
  │──────────────────────────────────>│                              │
  │                                   │                              │
  │                    6. Verify:      │                              │
  │                    - STARK proof   │                              │
  │                    - NoteId binding│                              │
  │                    - Payment details                              │
  │                                   │                              │
  │                    7. Submit       │                              │
  │                    ProvenTransaction                              │
  │                                   │─────────────────────────────>│
  │                                   │     Block confirmed          │
  │                                   │<─────────────────────────────│
  │   200 OK + Resource               │                              │
  │<──────────────────────────────────│                              │
  │                                   │                              │
  │                    8. Deliver full │                              │
  │                    note to recipient                              │
  │                    (off-chain transport)                          │
```

### 5.2 Facilitator Verification (Cryptographic Binding)

The facilitator does NOT need to trust the client. The binding is cryptographic:

```rust
// Facilitator verification pseudocode
fn verify_private_payment(
    proven_tx: &ProvenTransaction,
    client_provided_note: &Note,
    expected_recipient: &str,
    expected_faucet: &str,
    expected_amount: u64,
) -> Result<(), VerifyError> {
    // 1. Verify STARK proof
    let verifier = TransactionVerifier::new(96);
    verifier.verify(proven_tx)?;

    // 2. Compute NoteId from client-provided note
    let computed_note_id = client_provided_note.id();

    // 3. Find matching OutputNote::Header in ProvenTransaction
    let found = proven_tx.output_notes().iter().any(|on| on.id() == computed_note_id);
    assert!(found, "Note ID must match a ProvenTransaction output");

    // 4. Verify payment details in the full note
    //    (same verification as for public notes)
    let p2id_root = WellKnownNote::P2ID.script_root();
    assert_eq!(client_provided_note.recipient().script().root(), p2id_root);

    let inputs = client_provided_note.recipient().inputs().values();
    let target = AccountId::new_unchecked([inputs[1], inputs[0]]);
    assert_eq!(target, AccountId::from_hex(expected_recipient)?);

    for asset in client_provided_note.assets().iter_fungible() {
        if asset.faucet_id() == AccountId::from_hex(expected_faucet)?
            && asset.amount() >= expected_amount
        {
            return Ok(());
        }
    }

    Err(VerifyError::PaymentNotFound)
}
```

**Why this is secure**: The `NoteId` is a cryptographic hash commitment to the full note content. The client cannot forge a note that matches the NoteId in the ProvenTransaction without knowing the note's serial number (which was generated during execution). The STARK proof guarantees the ProvenTransaction was validly executed.

### 5.3 Payload Changes

```rust
// Current MidenExactPayload
pub struct MidenExactPayload {
    pub from: MidenAccountAddress,
    pub proven_transaction: String,      // hex
    pub transaction_id: String,          // hex
    pub transaction_inputs: String,      // hex
}

// Phase 2: add optional note_data for private notes
pub struct MidenExactPayload {
    pub from: MidenAccountAddress,
    pub proven_transaction: String,      // hex
    pub transaction_id: String,          // hex
    pub transaction_inputs: String,      // hex
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note_data: Option<String>,       // hex-encoded full Note (for private notes)
}
```

### 5.4 Privacy Guarantees

| Observer | What they see |
|----------|---------------|
| **Chain observers** | Note hash only. Cannot determine recipient, amount, or faucet. |
| **Miden node operator** | ProvenTransaction + TransactionInputs (training wheels phase). Sees note hash. |
| **Facilitator** | Full note data (recipient, amount, faucet). Same as a payment processor. |
| **Other users** | Nothing. Nullifier is unlinkable without note details. |

This is equivalent to how Stripe/PayPal work: the payment processor sees the transaction, but the public doesn't.

---

## 6. Future: Phase 3 with Encrypted Notes

### NoteType::Encrypted + Facilitator Off-chain Verification

Phase 3 adds recipient self-discovery: the note is encrypted on-chain so the recipient can find and decrypt it without off-chain delivery.

### 6.1 Changes from Phase 2

1. Use `NoteType::Encrypted` instead of `NoteType::Private`
2. Encrypt note details with recipient's `SealingKey` from their `Address`
3. Attach ciphertext as `NoteAttachment::Array` in note metadata
4. `OutputNote::Full` preserved in ProvenTransaction (not shrunk)
5. On-chain: ciphertext visible but undecryptable without recipient's key
6. Recipient syncs chain, finds encrypted note, decrypts with `UnsealingKey`

### 6.2 Facilitator Verification

Same as Phase 2 — facilitator receives full note data off-chain via x402 payload. The encrypted on-chain data is irrelevant to the facilitator.

### 6.3 Implementation Requirements

```rust
// Requires recipient's Address with SealingKey
let recipient_address = Address::from_hex(recipient_address_str)?;
let sealing_key = recipient_address.routing_params()
    .encryption_key()
    .ok_or("Recipient has no encryption key")?;

// Encrypt NoteDetails
let note_details = NoteDetails::new(assets, recipient);
let ciphertext = sealing_key.seal_bytes(rng, note_details.to_bytes())?;

// Create note with encrypted attachment
let attachment = NoteAttachment::array(ciphertext)?;
let note = create_p2id_note(
    sender, target, assets,
    NoteType::Encrypted,
    attachment,
    rng,
)?;
```

### 6.4 Prerequisites

- Recipient must have an `Address` with `SealingKey` (Miden v0.12+ feature)
- `NoteAttachment::Array` encryption API must be stable
- Note transport layer integration for encrypted note discovery

**Difficulty: Medium** — requires ~200 LOC, but depends on Miden's encryption API stability.

---

## 7. Alternative Approaches (Evaluated and Deferred)

### 7.1 ZK Proof of Payment (Deferred to Phase 4+)

A client-generated secondary ZK proof attesting to payment properties without revealing details. This would give **privacy even from the facilitator**.

```
Client proves: "There exists a note N in my ProvenTransaction such that:
  - N is a P2ID note
  - N pays ≥ X tokens of faucet F to recipient R
  - N's NoteId appears in the ProvenTransaction outputs
WITHOUT revealing N's content to the verifier."
```

This is theoretically possible using Miden VM as the ZK prover (Miden programs ARE ZK proofs). However:

- **No existing patterns** for this on Miden
- Requires designing a custom Miden program that proves note properties
- The "inner proof" (STARK over the payment check) would need to be verified by the facilitator
- **Complexity: Very High** — estimated 500+ LOC, deep Miden VM knowledge needed
- **Deferred** until the ecosystem matures and demand justifies the effort

### 7.2 Custom Note Script (Deferred)

Embed facilitator authorization in the note script itself:

```masm
# P2ID note script with facilitator authorization
# Note can only be consumed if facilitator account has set an approval flag
proc.verify_facilitator_approval
    # Read facilitator account storage slot for approval
    # If approved, allow consumption; otherwise, fail
end
```

This moves verification on-chain but:
- Changes the P2ID semantics (no longer standard `WellKnownNote::P2ID`)
- Requires a facilitator account on-chain with storage
- **Difficulty: Hard** — custom MASM, non-standard note scripts
- **Deferred** — too much architectural change for limited benefit

### 7.3 Commit-Reveal (Not Recommended)

1. Client commits payment hash
2. Facilitator grants access
3. Client reveals payment details

Problems:
- Two round trips (latency)
- Doesn't prevent front-running between commit and reveal
- No advantage over Trusted Facilitator approach
- **Not recommended**

### 7.4 Selective Disclosure (Not Feasible)

Reveal only amount + recipient, hide sender:

- `NoteInputs` has no selective disclosure mechanism (all-or-nothing)
- `NoteRecipient` digest is a single hash — no Merkle path for partial reveal
- Would require protocol-level changes to Miden
- **Not feasible** with current primitives

---

## 8. Implementation Roadmap

### Phase 1: Current (NoteType::Public) — Complete

- All payment data visible on-chain
- Facilitator verifies `OutputNote::Full` directly
- **Status: Implemented and tested on testnet**

### Phase 2: Trusted Facilitator + NoteType::Private — Next

**Changes required:**

1. **`MidenSignerLike` trait** — add `note_type` parameter or new method:
   ```rust
   async fn create_and_prove_p2id_private(
       &self, recipient: &str, faucet_id: &str, amount: u64,
   ) -> Result<(String, String, String, String), X402Error>;
   //                                    ^ note_data_hex
   ```

2. **`MidenClientSigner`** — extract full note from `TransactionResult.created_notes()` before proving, serialize it

3. **`MidenExactPayload`** — add optional `note_data: Option<String>` field

4. **Facilitator verification** — if `note_data` present:
   - Deserialize full `Note`
   - Compute `NoteId`, verify matches `OutputNote::Header` in ProvenTransaction
   - Inspect note for payment details (same logic as public verification)

5. **Note delivery** — after facilitator submits, deliver full note to recipient (via Miden's P2P transport or direct channel)

**Estimated effort: ~100 LOC across 4 files**
**Difficulty: Easy**

### Phase 3: NoteType::Encrypted — Future

- Requires stable SealingKey API and Address encryption support
- Adds recipient self-discovery (no off-chain note delivery needed)
- Facilitator verification unchanged from Phase 2
- **Estimated effort: ~200 LOC**
- **Difficulty: Medium**

### Phase 4: ZK Proof of Payment — Long-term

- Full privacy from facilitator
- Requires custom Miden VM program for payment property proofs
- **Estimated effort: 500+ LOC, deep ZK expertise**
- **Difficulty: Very Hard**

---

## 9. Sources

### Miden Documentation
- [Miden Privacy Blog](https://miden.xyz/resource/blog/privacy) — "Privacy Simply Scales Better"
- [Miden Testnet v0.12 Blog](https://miden.xyz/resource/blog/testnet-november-2025) — Note encryption, transport layer
- [Miden Private Multisig / PSM](https://miden.xyz/resource/blog/private-multisig) — Private State Management pattern

### Miden Protocol Source (v0.13.3)
- `src/note/note_type.rs` — NoteType enum: Public, Private, Encrypted
- `src/note/metadata.rs` — NoteMetadata: sender, type, tag, attachment (always public)
- `src/note/recipient.rs` — NoteRecipient: serial_num, script, inputs, digest
- `src/note/attachment.rs` — NoteAttachment: encrypted note delivery mechanism
- `src/transaction/outputs.rs` — OutputNote::shrink(): Private→Header, Encrypted→Full
- `src/address/routing_parameters.rs` — SealingKey for note encryption

### Spark / compolabs
- [GitHub: compolabs/spark-miden-v1](https://github.com/compolabs/spark-miden-v1)
- [Polygon Blog: Composability Labs / Spark](https://polygon.technology/blog/miden-pioneers-composability-labs-is-building-spark-a-superfast-onchain-clob-with-a-state-minimized-approach)
- [Medium: Privacy-Focused CLOB on Miden](https://medium.com/sprkfi/developing-a-privacy-focused-decentralized-order-book-exchange-on-polygon-miden-95547119543f)
- [HackMD: Order Book Designs on Miden](https://hackmd.io/kEvttsjSS6WS3rALMyF_tA)
- [HackMD: Potential CLOB on Miden](https://hackmd.io/@Domi2000/BkWevSirn)

### Miden Architecture
- [Polygon Blog: Miden State Model](https://polygon.technology/blog/polygon-miden-state-model)
- [Polygon Blog: Miden Transaction Model](https://polygon.technology/blog/polygon-miden-transaction-model-2)
- [HackMD: Technical Analysis of Miden Transactions](https://hackmd.io/@rishotics/SJkmOpCbR)
- [GitHub: 0xMiden/miden-base](https://github.com/0xMiden/miden-base)

### x402 Testnet Verification
- `tests/e2e_testnet.rs:e2e_private_p2id_transfer` — Our test confirming Private notes produce `OutputNote::Header`
- Private TX: `0x5adf5ccb59184982f5a73528b41762edf88e253351c6ee8e7b06e55effe087c4`
