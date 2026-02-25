//! Privacy mode configuration for x402 Miden payments.
//!
//! This module defines the [`PrivacyMode`] enum and provides note verification
//! functions for each mode:
//!
//! - **Public**: Notes are fully visible on-chain (default, backward-compatible)
//! - **TrustedFacilitator**: Notes are private on-chain; full note data is shared
//!   with the facilitator off-chain via the x402 payload

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// Privacy mode for x402 Miden payments.
///
/// Controls how payment notes are created and verified:
///
/// - `Public` (default): `NoteType::Public` — full note data on-chain.
///   The facilitator verifies by inspecting `OutputNote::Full` in the proven transaction.
///
/// - `TrustedFacilitator`: `NoteType::Private` — only note hash on-chain.
///   The client shares the full note data off-chain via the `noteData` payload field.
///   The facilitator verifies the cryptographic NoteId binding between the full note
///   and the on-chain commitment, then checks payment details from the full note.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PrivacyMode {
    /// Public notes — full data visible on-chain (default).
    #[default]
    Public,
    /// Private notes with trusted facilitator — only hash on-chain,
    /// full note shared off-chain with facilitator.
    TrustedFacilitator,
}

impl fmt::Display for PrivacyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PrivacyMode::Public => write!(f, "public"),
            PrivacyMode::TrustedFacilitator => write!(f, "trusted_facilitator"),
        }
    }
}

impl FromStr for PrivacyMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "public" => Ok(PrivacyMode::Public),
            "trusted_facilitator" => Ok(PrivacyMode::TrustedFacilitator),
            other => Err(format!("unknown privacy mode: '{other}'")),
        }
    }
}

impl Serialize for PrivacyMode {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for PrivacyMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        PrivacyMode::from_str(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(feature = "miden-native")]
mod public;
#[cfg(feature = "miden-native")]
pub use public::verify_public_note;

#[cfg(feature = "miden-native")]
mod trusted;
#[cfg(feature = "miden-native")]
pub use trusted::verify_trusted_facilitator_note;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_privacy_mode_default() {
        assert_eq!(PrivacyMode::default(), PrivacyMode::Public);
    }

    #[test]
    fn test_privacy_mode_display() {
        assert_eq!(PrivacyMode::Public.to_string(), "public");
        assert_eq!(
            PrivacyMode::TrustedFacilitator.to_string(),
            "trusted_facilitator"
        );
    }

    #[test]
    fn test_privacy_mode_from_str() {
        assert_eq!(
            "public".parse::<PrivacyMode>().unwrap(),
            PrivacyMode::Public
        );
        assert_eq!(
            "trusted_facilitator".parse::<PrivacyMode>().unwrap(),
            PrivacyMode::TrustedFacilitator
        );
        assert!("unknown".parse::<PrivacyMode>().is_err());
    }

    #[test]
    fn test_privacy_mode_serde_roundtrip() {
        for mode in [PrivacyMode::Public, PrivacyMode::TrustedFacilitator] {
            let json = serde_json::to_string(&mode).unwrap();
            let recovered: PrivacyMode = serde_json::from_str(&json).unwrap();
            assert_eq!(mode, recovered);
        }
    }

    #[test]
    fn test_privacy_mode_serde_values() {
        assert_eq!(
            serde_json::to_string(&PrivacyMode::Public).unwrap(),
            "\"public\""
        );
        assert_eq!(
            serde_json::to_string(&PrivacyMode::TrustedFacilitator).unwrap(),
            "\"trusted_facilitator\""
        );
    }
}
