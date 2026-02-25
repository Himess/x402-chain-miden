//! Trusted facilitator note verification for x402 Miden payments.
//!
//! Verifies P2ID payment notes that are private on-chain
//! (`NoteType::Private` -> `OutputNote::Header`). The full note data
//! is provided off-chain via the x402 payload's `noteData` field.

use miden_protocol::account::AccountId;
use miden_protocol::note::Note;
use miden_protocol::transaction::ProvenTransaction;
use miden_protocol::utils::serde::Deserializable;
use miden_standards::note::WellKnownNote;

use crate::v2_miden_exact::types::MidenExactError;

/// Verifies a private P2ID note using off-chain note data.
///
/// 1. Decodes the hex note data and deserializes the full [`Note`]
/// 2. Computes the note's ID and verifies it matches an output note
///    in the proven transaction (NoteId binding)
/// 3. Verifies the note is a P2ID note targeting the required recipient
/// 4. Checks the note contains the required faucet and amount
pub fn verify_trusted_facilitator_note(
    proven_tx: &ProvenTransaction,
    note_data_hex: &str,
    required_recipient: AccountId,
    required_faucet: AccountId,
    required_amount: u64,
) -> Result<(), MidenExactError> {
    // 1. Decode and deserialize the full note
    let note_bytes = hex::decode(note_data_hex).map_err(|e| {
        MidenExactError::NoteBindingFailed(format!("Invalid hex in note_data: {e}"))
    })?;

    let note = Note::read_from_bytes(&note_bytes).map_err(|e| {
        MidenExactError::NoteBindingFailed(format!("Failed to deserialize Note: {e}"))
    })?;

    // 2. Verify NoteId binding — the note's ID must appear in the proven transaction's outputs
    let note_id = note.id();
    let id_matches = proven_tx
        .output_notes()
        .iter()
        .any(|output_note| output_note.id() == note_id);

    if !id_matches {
        return Err(MidenExactError::NoteBindingFailed(format!(
            "Note ID {note_id} does not match any output note in the proven transaction"
        )));
    }

    // 3. Verify P2ID script root
    let p2id_script_root = WellKnownNote::P2ID.script_root();
    let script_root = note.recipient().script().root();
    if script_root != p2id_script_root {
        return Err(MidenExactError::NoteBindingFailed(
            "Note is not a P2ID note (script root mismatch)".to_string(),
        ));
    }

    // 4. Extract and verify target account
    let inputs = note.recipient().inputs().values();
    if inputs.len() < 2 {
        return Err(MidenExactError::NoteBindingFailed(
            "P2ID note has insufficient inputs".to_string(),
        ));
    }
    let target = AccountId::new_unchecked([inputs[1], inputs[0]]);

    if target != required_recipient {
        return Err(MidenExactError::RecipientMismatch {
            expected: format!("{required_recipient}"),
            got: format!("{target}"),
        });
    }

    // 5. Check assets for the required fungible asset
    let mut payment_found = false;
    for fungible in note.assets().iter_fungible() {
        if fungible.faucet_id() == required_faucet && fungible.amount() >= required_amount {
            payment_found = true;
            break;
        }
    }

    if !payment_found {
        return Err(MidenExactError::PaymentNotFound(
            "Off-chain note does not contain the required faucet and amount".to_string(),
        ));
    }

    Ok(())
}
