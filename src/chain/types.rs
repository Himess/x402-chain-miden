//! Wire format types for Miden chain interactions.
//!
//! This module provides types that handle serialization and deserialization
//! of Miden-specific values in the x402 protocol wire format.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt::{Display, Formatter};
use std::str::FromStr;
use x402_types::chain::ChainId;

/// The CAIP-2 namespace for Miden chains.
pub const MIDEN_NAMESPACE: &str = "miden";

// ============================================================================
// MidenAccountAddress
// ============================================================================

/// A Miden account ID that serializes as a hex string.
///
/// Miden account IDs are 120-bit (15 bytes) identifiers. This wrapper
/// ensures consistent serialization in the x402 protocol wire format.
///
/// # Example
///
/// ```
/// use x402_chain_miden::chain::MidenAccountAddress;
///
/// // 15 bytes = 30 hex chars
/// let addr: MidenAccountAddress = "0xabcdef1234567890abcdef12345678".parse().unwrap();
/// assert!(addr.to_string().starts_with("0x"));
/// ```
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct MidenAccountAddress(Vec<u8>);

/// The expected byte length of a Miden account ID (120 bits = 15 bytes).
pub const MIDEN_ACCOUNT_ID_BYTE_LEN: usize = 15;

impl MidenAccountAddress {
    /// Creates a new MidenAccountAddress from raw bytes.
    ///
    /// # Errors
    ///
    /// Returns an error if the input is not exactly 15 bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, MidenAddressParseError> {
        if bytes.len() != MIDEN_ACCOUNT_ID_BYTE_LEN {
            return Err(MidenAddressParseError::InvalidLength {
                expected: MIDEN_ACCOUNT_ID_BYTE_LEN,
                got: bytes.len(),
            });
        }
        Ok(Self(bytes))
    }

    /// Returns the raw bytes of the account ID.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Returns the hex-encoded account ID with 0x prefix.
    pub fn to_hex(&self) -> String {
        format!("0x{}", hex::encode(&self.0))
    }
}

impl FromStr for MidenAccountAddress {
    type Err = MidenAddressParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let s = s.strip_prefix("0x").unwrap_or(s);
        let bytes =
            hex::decode(s).map_err(|e| MidenAddressParseError::InvalidHex(e.to_string()))?;
        if bytes.len() != MIDEN_ACCOUNT_ID_BYTE_LEN {
            return Err(MidenAddressParseError::InvalidLength {
                expected: MIDEN_ACCOUNT_ID_BYTE_LEN,
                got: bytes.len(),
            });
        }
        Ok(Self(bytes))
    }
}

impl Display for MidenAccountAddress {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{}", hex::encode(&self.0))
    }
}

impl Serialize for MidenAccountAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for MidenAccountAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

/// Conversion methods for interoperating with the miden-protocol `AccountId` type.
///
/// These methods are only available when the `miden-native` feature is enabled.
#[cfg(feature = "miden-native")]
impl MidenAccountAddress {
    /// Converts this address to a miden-protocol `AccountId`.
    ///
    /// Parses the hex-encoded account ID using `AccountId::from_hex`.
    pub fn to_account_id(&self) -> Result<miden_protocol::account::AccountId, MidenAddressParseError> {
        let hex_str = self.to_hex();
        miden_protocol::account::AccountId::from_hex(&hex_str)
            .map_err(|e| MidenAddressParseError::InvalidAccountId(e.to_string()))
    }

    /// Creates a `MidenAccountAddress` from a miden-protocol `AccountId`.
    pub fn from_account_id(id: miden_protocol::account::AccountId) -> Self {
        let hex_str = id.to_hex();
        // to_hex returns "0x..." prefixed string
        hex_str.parse().expect("AccountId::to_hex always produces valid hex")
    }
}

/// Error returned when parsing a Miden account address.
#[derive(Debug, thiserror::Error)]
pub enum MidenAddressParseError {
    /// The hex string is invalid.
    #[error("Invalid hex: {0}")]
    InvalidHex(String),

    /// The byte length is wrong (expected 15 bytes / 120 bits).
    #[error("Invalid length: expected {expected} bytes, got {got}")]
    InvalidLength { expected: usize, got: usize },

    /// The account ID is invalid (wrong length, checksum, etc.).
    #[cfg(feature = "miden-native")]
    #[error("Invalid account ID: {0}")]
    InvalidAccountId(String),
}

// ============================================================================
// MidenChainReference
// ============================================================================

/// A Miden chain reference (e.g., `testnet` or `mainnet`).
///
/// Combined with the `miden` namespace, this forms a CAIP-2 chain ID
/// like `miden:testnet` or `miden:mainnet`.
///
/// # Example
///
/// ```
/// use x402_chain_miden::chain::MidenChainReference;
/// use x402_types::chain::ChainId;
///
/// let testnet = MidenChainReference::testnet();
/// let chain_id: ChainId = testnet.into();
/// assert_eq!(chain_id.to_string(), "miden:testnet");
/// ```
#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub struct MidenChainReference(String);

impl MidenChainReference {
    /// Creates a new chain reference from a string.
    pub fn new(reference: impl Into<String>) -> Self {
        Self(reference.into())
    }

    /// Returns the Miden testnet chain reference.
    pub fn testnet() -> Self {
        Self("testnet".to_string())
    }

    /// Returns the Miden mainnet chain reference.
    pub fn mainnet() -> Self {
        Self("mainnet".to_string())
    }

    /// Converts this chain reference to a CAIP-2 [`ChainId`].
    pub fn as_chain_id(&self) -> ChainId {
        ChainId::new(MIDEN_NAMESPACE, &self.0)
    }

    /// Returns the inner reference string.
    pub fn inner(&self) -> &str {
        &self.0
    }
}

impl Display for MidenChainReference {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<MidenChainReference> for ChainId {
    fn from(value: MidenChainReference) -> Self {
        ChainId::new(MIDEN_NAMESPACE, value.0)
    }
}

impl From<&MidenChainReference> for ChainId {
    fn from(value: &MidenChainReference) -> Self {
        ChainId::new(MIDEN_NAMESPACE, &value.0)
    }
}

impl TryFrom<ChainId> for MidenChainReference {
    type Error = MidenChainReferenceFormatError;

    fn try_from(value: ChainId) -> Result<Self, Self::Error> {
        if value.namespace != MIDEN_NAMESPACE {
            return Err(MidenChainReferenceFormatError::InvalidNamespace(
                value.namespace,
            ));
        }
        Ok(MidenChainReference(value.reference))
    }
}

impl TryFrom<&ChainId> for MidenChainReference {
    type Error = MidenChainReferenceFormatError;

    fn try_from(value: &ChainId) -> Result<Self, Self::Error> {
        if value.namespace != MIDEN_NAMESPACE {
            return Err(MidenChainReferenceFormatError::InvalidNamespace(
                value.namespace.clone(),
            ));
        }
        Ok(MidenChainReference(value.reference.clone()))
    }
}

impl TryFrom<&str> for MidenChainReference {
    type Error = MidenChainReferenceFormatError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "testnet" | "mainnet" => Ok(MidenChainReference(value.to_string())),
            _ => Err(MidenChainReferenceFormatError::InvalidReference(
                value.to_string(),
            )),
        }
    }
}

/// Error returned when converting a [`ChainId`] to a [`MidenChainReference`].
#[derive(Debug, thiserror::Error)]
pub enum MidenChainReferenceFormatError {
    /// The chain ID namespace is not `miden`.
    #[error("Invalid namespace {0}, expected miden")]
    InvalidNamespace(String),
    /// The reference string is not a known Miden network.
    #[error("Invalid reference {0}, expected testnet or mainnet")]
    InvalidReference(String),
}

// ============================================================================
// MidenTokenDeployment
// ============================================================================

/// Information about a token (faucet) deployment on a Miden chain.
///
/// On Miden, tokens are issued by faucet accounts. The faucet's account ID
/// serves as the token identifier (analogous to an ERC-20 contract address
/// on EVM chains).
#[derive(Debug, Clone, Eq, PartialEq, Hash)]
pub struct MidenTokenDeployment {
    /// The chain this faucet is deployed on.
    pub chain_reference: MidenChainReference,
    /// The faucet account ID (token issuer).
    pub faucet_id: MidenAccountAddress,
    /// Number of decimal places for the token (e.g., 6 for USDC-equivalent).
    pub decimals: u8,
}

/// A token amount paired with its deployment information.
#[derive(Debug, Clone)]
pub struct MidenDeployedTokenAmount {
    /// The amount in the token's smallest unit.
    pub amount: u64,
    /// The token deployment this amount refers to.
    pub token: MidenTokenDeployment,
}

impl MidenTokenDeployment {
    /// Creates a token amount from a raw value.
    ///
    /// The value should already be in the token's smallest unit.
    pub fn amount(&self, v: u64) -> MidenDeployedTokenAmount {
        MidenDeployedTokenAmount {
            amount: v,
            token: self.clone(),
        }
    }

    /// Parses a human-readable amount string into token units.
    ///
    /// Accepts formats like `"10.50"`, `"1000"`, etc.
    /// The amount is scaled by the token's decimal places.
    ///
    /// # Errors
    ///
    /// Returns an error if the input cannot be parsed or exceeds u64 range.
    pub fn parse(&self, v: &str) -> Result<MidenDeployedTokenAmount, MidenAmountParseError> {
        let parts: Vec<&str> = v.split('.').collect();
        let (whole, frac) = match parts.len() {
            1 => (parts[0], ""),
            2 => (parts[0], parts[1]),
            _ => return Err(MidenAmountParseError::InvalidFormat(v.to_string())),
        };

        let frac_len = frac.len() as u32;
        if frac_len > self.decimals as u32 {
            return Err(MidenAmountParseError::TooManyDecimals {
                got: frac_len,
                max: self.decimals,
            });
        }

        let whole_val: u64 = whole
            .parse()
            .map_err(|_| MidenAmountParseError::InvalidFormat(v.to_string()))?;
        let frac_val: u64 = if frac.is_empty() {
            0
        } else {
            frac.parse()
                .map_err(|_| MidenAmountParseError::InvalidFormat(v.to_string()))?
        };

        let scale = 10u64.pow(self.decimals as u32);
        let frac_scale = 10u64.pow(self.decimals as u32 - frac_len);

        let total = whole_val
            .checked_mul(scale)
            .and_then(|w| w.checked_add(frac_val.checked_mul(frac_scale)?))
            .ok_or(MidenAmountParseError::Overflow)?;

        Ok(MidenDeployedTokenAmount {
            amount: total,
            token: self.clone(),
        })
    }
}

/// Error returned when parsing a token amount.
#[derive(Debug, thiserror::Error)]
pub enum MidenAmountParseError {
    /// The input string is not a valid number.
    #[error("Invalid amount format: {0}")]
    InvalidFormat(String),
    /// Too many decimal places for the token.
    #[error("Too many decimal places: got {got}, max {max}")]
    TooManyDecimals { got: u32, max: u8 },
    /// The resulting amount overflows u64.
    #[error("Amount overflow")]
    Overflow,
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_miden_chain_reference_to_chain_id() {
        let testnet = MidenChainReference::testnet();
        let chain_id: ChainId = testnet.into();
        assert_eq!(chain_id.namespace, "miden");
        assert_eq!(chain_id.reference, "testnet");
        assert_eq!(chain_id.to_string(), "miden:testnet");
    }

    #[test]
    fn test_chain_id_to_miden_chain_reference() {
        let chain_id = ChainId::new("miden", "mainnet");
        let reference = MidenChainReference::try_from(chain_id).unwrap();
        assert_eq!(reference.inner(), "mainnet");
    }

    #[test]
    fn test_chain_id_wrong_namespace() {
        let chain_id = ChainId::new("eip155", "8453");
        let result = MidenChainReference::try_from(chain_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_miden_address_roundtrip() {
        let hex_str = "0xabcdef1234567890abcdef12345678"; // 15 bytes
        let addr: MidenAccountAddress = hex_str.parse().unwrap();
        assert_eq!(addr.to_string(), hex_str);
    }

    #[test]
    fn test_miden_address_without_prefix() {
        let addr: MidenAccountAddress = "abcdef1234567890abcdef12345678".parse().unwrap();
        assert_eq!(addr.to_string(), "0xabcdef1234567890abcdef12345678");
    }

    #[test]
    fn test_miden_address_rejects_wrong_length() {
        // Too short (3 bytes)
        assert!("abcdef".parse::<MidenAccountAddress>().is_err());
        // Too long (16 bytes)
        assert!("abcdef1234567890abcdef1234567890".parse::<MidenAccountAddress>().is_err());
    }

    #[test]
    fn test_token_deployment_amount() {
        let deployment = MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            faucet_id: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            decimals: 6,
        };
        let amount = deployment.amount(1_000_000);
        assert_eq!(amount.amount, 1_000_000);
    }

    #[test]
    fn test_token_deployment_parse_whole() {
        let deployment = MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            faucet_id: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            decimals: 6,
        };
        let amount = deployment.parse("100").unwrap();
        assert_eq!(amount.amount, 100_000_000);
    }

    #[test]
    fn test_token_deployment_parse_with_decimals() {
        let deployment = MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            faucet_id: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            decimals: 6,
        };
        let amount = deployment.parse("1.50").unwrap();
        assert_eq!(amount.amount, 1_500_000);
    }

    #[test]
    fn test_token_deployment_parse_too_many_decimals() {
        let deployment = MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            faucet_id: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            decimals: 2,
        };
        let result = deployment.parse("1.234");
        assert!(result.is_err());
    }

    #[test]
    fn test_token_deployment_parse_smallest_unit() {
        let deployment = MidenTokenDeployment {
            chain_reference: MidenChainReference::testnet(),
            faucet_id: "0xaabbccddeeff00112233aabbccddee".parse().unwrap(),
            decimals: 6,
        };
        let amount = deployment.parse("0.000001").unwrap();
        assert_eq!(amount.amount, 1);
    }

    #[test]
    fn test_miden_address_serde_roundtrip() {
        let addr: MidenAccountAddress = "0xabcdef1234567890abcdef12345678".parse().unwrap();
        let json = serde_json::to_string(&addr).unwrap();
        assert_eq!(json, "\"0xabcdef1234567890abcdef12345678\"");
        let deserialized: MidenAccountAddress = serde_json::from_str(&json).unwrap();
        assert_eq!(addr, deserialized);
    }
}
