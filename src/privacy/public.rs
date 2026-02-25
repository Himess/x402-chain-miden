//! Public note verification for x402 Miden payments.
//!
//! Verifies P2ID payment notes that are fully visible on-chain
//! (`NoteType::Public` -> `OutputNote::Full`).

use miden_protocol::account::AccountId;
use miden_protocol::transaction::{OutputNote, ProvenTransaction};
use miden_standards::note::WellKnownNote;

use crate::v2_miden_exact::types::MidenExactError;

/// Verifies that a proven transaction contains a public P2ID note
/// paying the required recipient the required amount from the required faucet.
///
/// Iterates over `OutputNote::Full` variants in the proven transaction's
/// output notes, checking for a P2ID note matching all requirements.
pub fn verify_public_note(
    proven_tx: &ProvenTransaction,
    required_recipient: AccountId,
    required_faucet: AccountId,
    required_amount: u64,
) -> Result<(), MidenExactError> {
    let p2id_script_root = WellKnownNote::P2ID.script_root();
    let mut payment_found = false;

    for output_note in proven_tx.output_notes().iter() {
        if let OutputNote::Full(note) = output_note {
            let script_root = note.recipient().script().root();
            if script_root != p2id_script_root {
                continue;
            }

            let inputs = note.recipient().inputs().values();
            if inputs.len() < 2 {
                continue;
            }
            let target = AccountId::new_unchecked([inputs[1], inputs[0]]);

            if target != required_recipient {
                continue;
            }

            for fungible in note.assets().iter_fungible() {
                if fungible.faucet_id() == required_faucet && fungible.amount() >= required_amount {
                    payment_found = true;
                    break;
                }
            }

            if payment_found {
                break;
            }
        }
    }

    if !payment_found {
        return Err(MidenExactError::PaymentNotFound(
            "No P2ID output note found matching the required recipient, faucet, and amount. \
             Note: only NoteType::Public notes can be verified in public mode."
                .to_string(),
        ));
    }

    Ok(())
}
