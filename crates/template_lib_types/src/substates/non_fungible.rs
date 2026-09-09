//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use minicbor::{CborLen, Decode, Encode};
use tari_bor::BorTag;
use tari_template_abi::{
    EngineOp,
    call_engine,
    rust::{fmt, fmt::Display, prelude::*, str::FromStr, write},
};

use super::{BinaryTag, ComponentAddress, ResourceAddress, TemplateAddress};
use crate::{
    MaxString,
    address_prefixes,
    constants::{
        CALLER_COMPONENT_RESOURCE_ADDRESS,
        DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS,
        PUBLIC_IDENTITY_RESOURCE_ADDRESS,
    },
    crypto::RistrettoPublicKeyBytes,
    hex::{fixed_bytes_from_hex, write_hex_fmt},
};

const NON_FUNGIBLE_ID_STR_MAX_LEN: usize = 64;

/// The unique identification of a non-fungible token inside it's parent resource
#[derive(Debug, Clone, Ord, PartialOrd, PartialEq, Eq, Hash, Encode, Decode, CborLen)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub enum NonFungibleId {
    #[n(0)]
    U256(
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        #[cfg_attr(feature = "serde", serde(with = "crate::serde_helpers::fixed_hex"))]
        #[n(0)]
        #[cbor(with = "minicbor::bytes")]
        [u8; 32],
    ),
    #[n(1)]
    String(
        #[cfg_attr(feature = "ts", ts(type = "string"))]
        #[n(0)]
        MaxString<NON_FUNGIBLE_ID_STR_MAX_LEN>,
    ),
    #[n(2)]
    Uint32(#[n(0)] u32),
    #[n(3)]
    Uint64(
        #[cfg_attr(feature = "ts", ts(type = "number"))]
        #[n(0)]
        u64,
    ),
}

impl NonFungibleId {
    pub fn random() -> Self {
        let uuid = call_engine(EngineOp::GenerateUniqueId, &());
        Self::U256(uuid)
    }

    pub fn from_u256(id: [u8; 32]) -> Self {
        Self::U256(id)
    }

    pub fn from_u32(id: u32) -> Self {
        Self::Uint32(id)
    }

    pub fn from_u64(id: u64) -> Self {
        Self::Uint64(id)
    }

    pub const fn from_public_key(public_key: RistrettoPublicKeyBytes) -> Self {
        Self::U256(public_key.into_array())
    }

    /// Creates a NonFungibleId from a string, the string must be between 1 and 64 characters long. It can contain any
    /// UTF-8 character. Panics if the string is empty or longer than 64 characters.
    pub fn from_string<T: Into<String>>(id: T) -> Self {
        // Avoid long strings in WASM to reduce bin size
        #[cfg(not(target_arch = "wasm32"))]
        const EXPECT_MSG: &str = "Invariant violated: String cannot be empty or longer than 64 characters";
        #[cfg(target_arch = "wasm32")]
        const EXPECT_MSG: &str = "NFTSTRLEN";

        Self::try_from_string(id).expect(EXPECT_MSG)
    }

    pub fn try_from_string<T: Into<String>>(id: T) -> Result<Self, ParseNonFungibleIdError> {
        let id = id.into();
        validate_nft_id_str(&id)?;
        // SAFETY: length checked
        Ok(NonFungibleId::String(unsafe { MaxString::new_unchecked(id) }))
    }

    /// A string in one of the following formats
    /// - uuid_736bab0c3af393a0423c578ddcf7e19b81086f6ecbbc148713e95da75ef8171d
    /// - str_my_special_nft_name
    /// - u32_1234
    /// - u64_1234
    pub fn to_canonical_string(&self) -> String {
        let type_name = self.type_name();
        let mut s = String::with_capacity(type_name.len() + 1 + self.str_repr_len());
        s.push_str(self.type_name());
        s.push('_');

        match self {
            NonFungibleId::U256(uuid) => {
                // PANIC: the length of the string is pre-allocated and the function writes exactly 64 characters, so
                // this cannot fail
                write_hex_fmt(&mut s, uuid).unwrap()
            },
            NonFungibleId::String(st) => {
                s.push_str(st);
            },
            NonFungibleId::Uint32(i) => {
                s.push_str(&i.to_string());
            },
            NonFungibleId::Uint64(i) => {
                s.push_str(&i.to_string());
            },
        }
        s
    }

    /// Returns the length of the string representation of the ID (without the type prefix)
    fn str_repr_len(&self) -> usize {
        fn count_digits(mut n: u64) -> usize {
            let mut l = 0;
            while n > 0 {
                n /= 10;
                l += 1;
            }
            l
        }
        match self {
            NonFungibleId::U256(_) => 64, // 32 bytes in hex
            NonFungibleId::String(s) => s.len(),
            NonFungibleId::Uint32(i) => {
                if *i == 0 {
                    return 1;
                }
                count_digits(u64::from(*i))
            },
            NonFungibleId::Uint64(i) => {
                if *i == 0 {
                    return 1;
                }
                // log10(i)
                count_digits(*i)
            },
        }
    }

    fn type_name(&self) -> &'static str {
        match self {
            NonFungibleId::U256(_) => "uuid",
            NonFungibleId::String(_) => "str",
            NonFungibleId::Uint32(_) => "u32",
            NonFungibleId::Uint64(_) => "u64",
        }
    }

    pub fn try_from_canonical_string(s: &str) -> Result<Self, ParseNonFungibleIdError> {
        let (id_type, id) = s.split_once('_').ok_or(ParseNonFungibleIdError::InvalidFormat)?;
        match id_type {
            "uuid" => Ok(NonFungibleId::U256(
                fixed_bytes_from_hex(id).map_err(|_| ParseNonFungibleIdError::InvalidUuid)?,
            )),
            "str" => {
                validate_nft_id_str(id)?;
                // SAFETY: length checked
                Ok(NonFungibleId::String(unsafe {
                    MaxString::new_unchecked(id.to_string())
                }))
            },
            "u32" => Ok(NonFungibleId::Uint32(
                id.parse().map_err(|_| ParseNonFungibleIdError::InvalidUint32)?,
            )),
            "u64" => Ok(NonFungibleId::Uint64(
                id.parse().map_err(|_| ParseNonFungibleIdError::InvalidUint64)?,
            )),
            _ => Err(ParseNonFungibleIdError::InvalidType),
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            NonFungibleId::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u32(&self) -> Option<u32> {
        match self {
            NonFungibleId::Uint32(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            NonFungibleId::Uint64(i) => Some(*i),
            _ => None,
        }
    }

    pub fn as_u256(&self) -> Option<&[u8; 32]> {
        match self {
            NonFungibleId::U256(i) => Some(i),
            _ => None,
        }
    }
}

fn validate_nft_id_str(s: &str) -> Result<(), ParseNonFungibleIdError> {
    if s.is_empty() || s.len() > NON_FUNGIBLE_ID_STR_MAX_LEN {
        return Err(ParseNonFungibleIdError::InvalidStringLength);
    }
    Ok(())
}

impl Display for NonFungibleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NonFungibleId::U256(v) => {
                write!(f, "uuid_")?;
                write_hex_fmt(f, v)
            },
            NonFungibleId::String(s) => write!(f, "str_{}", s),
            NonFungibleId::Uint32(v) => write!(f, "u32_{}", v),
            NonFungibleId::Uint64(v) => write!(f, "u64_{}", v),
        }
    }
}

const TAG: u64 = BinaryTag::NonFungibleAddress.as_u64();

/// The unique identifier of a non-fungible index in the Tari network
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct NonFungibleAddress(BorTag<NonFungibleAddressContents, TAG>);

/// A NonFungibleId namespaced by a ResourceAddress.
#[derive(Debug, Clone, Encode, Decode, CborLen, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cfg_attr(feature = "borsh", derive(borsh::BorshSerialize, borsh::BorshDeserialize))]
pub struct NonFungibleAddressContents {
    #[n(0)]
    resource_address: ResourceAddress,
    #[n(1)]
    id: NonFungibleId,
}

impl NonFungibleAddress {
    pub const fn new(resource_address: ResourceAddress, id: NonFungibleId) -> Self {
        let inner = NonFungibleAddressContents { resource_address, id };
        Self(BorTag::new(inner))
    }

    pub fn resource_address(&self) -> &ResourceAddress {
        &self.0.inner().resource_address
    }

    pub fn id(&self) -> &NonFungibleId {
        &self.0.inner().id
    }

    pub fn from_public_key(public_key: RistrettoPublicKeyBytes) -> Self {
        Self::new(
            PUBLIC_IDENTITY_RESOURCE_ADDRESS,
            NonFungibleId::from_public_key(public_key),
        )
    }

    /// The virtual badge naming `address` as the component a frame is executing on behalf of. Stamped into a
    /// callee's authorization scope by the engine; see [`CALLER_COMPONENT_RESOURCE_ADDRESS`].
    pub fn caller_component_badge(address: ComponentAddress) -> Self {
        Self::new(
            CALLER_COMPONENT_RESOURCE_ADDRESS,
            NonFungibleId::U256(address.as_object_key().into_array()),
        )
    }

    /// The virtual badge naming `address` as the template of a frame's immediate caller. See
    /// [`DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS`].
    pub fn direct_caller_template_badge(address: TemplateAddress) -> Self {
        Self::new(
            DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS,
            NonFungibleId::U256(address.into_array()),
        )
    }

    pub fn to_public_key(&self) -> Option<RistrettoPublicKeyBytes> {
        if self.0.resource_address != PUBLIC_IDENTITY_RESOURCE_ADDRESS {
            return None;
        }
        match self.id() {
            NonFungibleId::U256(bytes) => RistrettoPublicKeyBytes::from_bytes(bytes).ok(),
            _ => None,
        }
    }
}

impl FromStr for NonFungibleAddress {
    type Err = ParseNonFungibleAddressError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        // nft_{resource_hex}_{type}_{id}

        let rest = s.strip_prefix("nft_").unwrap_or(s);
        let (resource, nft_rest) = rest
            .split_once('_')
            .ok_or(ParseNonFungibleAddressError::InvalidFormat)?;

        let resource_addr =
            ResourceAddress::from_hex(resource).map_err(|_| ParseNonFungibleAddressError::InvalidFormat)?;
        let nft_id = NonFungibleId::try_from_canonical_string(nft_rest)
            .map_err(|_| ParseNonFungibleAddressError::InvalidFormat)?;

        Ok(NonFungibleAddress::new(resource_addr, nft_id))
    }
}

impl From<NonFungibleAddressContents> for NonFungibleAddress {
    fn from(contents: NonFungibleAddressContents) -> Self {
        Self(BorTag::new(contents))
    }
}

impl From<RistrettoPublicKeyBytes> for NonFungibleAddress {
    fn from(public_key: RistrettoPublicKeyBytes) -> Self {
        Self::new(
            PUBLIC_IDENTITY_RESOURCE_ADDRESS,
            NonFungibleId::U256(public_key.into_array()),
        )
    }
}

impl Display for NonFungibleAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}_", address_prefixes::NON_FUNGIBLE)?;
        write_hex_fmt(f, self.resource_address().as_bytes())?;
        write!(f, "_{}", self.id())
    }
}

/// All the types of errors that can occur when parsing a non-fungible ID
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseNonFungibleIdError {
    InvalidFormat,
    InvalidType,
    InvalidString,
    InvalidStringLength,
    InvalidUuid,
    InvalidUint32,
    InvalidUint64,
}

impl Display for ParseNonFungibleIdError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// All the types of errors that can occur when parsing a non-fungible addresses
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseNonFungibleAddressError {
    InvalidFormat,
    InvalidResource(String),
    InvalidId(ParseNonFungibleIdError),
}

impl Display for ParseNonFungibleAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

#[cfg(test)]
mod tests {
    use std::{format, str::FromStr};

    use super::*;
    use crate::Hash32;

    mod try_from_string {
        use super::*;

        #[test]
        fn it_allows_a_valid_string() {
            NonFungibleId::try_from_string("_").unwrap();
            NonFungibleId::try_from_string("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789__").unwrap();
            NonFungibleId::try_from_string("hello123____!").unwrap();
            NonFungibleId::try_from_string("hello world").unwrap();
            NonFungibleId::try_from_string("❌nope❌").unwrap();
        }

        #[test]
        fn it_fails_for_an_invalid_string() {
            assert_eq!(
                NonFungibleId::try_from_string(""),
                Err(ParseNonFungibleIdError::InvalidStringLength)
            );
            assert_eq!(
                NonFungibleId::try_from_string(
                    "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                ),
                Err(ParseNonFungibleIdError::InvalidStringLength)
            );
        }
    }

    mod canonical_string {
        use super::*;

        #[test]
        fn it_generates_the_correct_length_for_ints() {
            assert_eq!(NonFungibleId::from_u32(0).str_repr_len(), 1);
            assert_eq!(NonFungibleId::from_u32(1).str_repr_len(), 1);
            assert_eq!(NonFungibleId::from_u32(10).str_repr_len(), 2);
            assert_eq!(NonFungibleId::from_u32(100).str_repr_len(), 3);
            assert_eq!(NonFungibleId::from_u32(1000).str_repr_len(), 4);
            assert_eq!(NonFungibleId::from_u32(12345).str_repr_len(), 5);
            assert_eq!(NonFungibleId::from_u32(100000).str_repr_len(), 6);
            assert_eq!(NonFungibleId::from_u32(1000000).str_repr_len(), 7);
            assert_eq!(NonFungibleId::from_u32(10000000).str_repr_len(), 8);
            assert_eq!(NonFungibleId::from_u32(100000000).str_repr_len(), 9);
            assert_eq!(NonFungibleId::from_u32(1000000000).str_repr_len(), 10);
            assert_eq!(
                NonFungibleId::from_u32(u32::MAX).str_repr_len(),
                u32::MAX.to_string().len()
            );

            assert_eq!(NonFungibleId::from_u64(0).str_repr_len(), 1);
            assert_eq!(NonFungibleId::from_u64(1).str_repr_len(), 1);
            assert_eq!(NonFungibleId::from_u64(10).str_repr_len(), 2);
            assert_eq!(NonFungibleId::from_u64(100).str_repr_len(), 3);
            assert_eq!(NonFungibleId::from_u64(1000).str_repr_len(), 4);
            assert_eq!(NonFungibleId::from_u64(123).str_repr_len(), 3);
            assert_eq!(
                NonFungibleId::from_u64(u64::MAX).str_repr_len(),
                u64::MAX.to_string().len()
            );
        }

        #[test]
        fn it_generates_correct_canonical_string() {
            // u32
            assert_eq!(NonFungibleId::from_u32(0).to_canonical_string(), "u32_0");
            assert_eq!(NonFungibleId::from_u32(100000).to_canonical_string(), "u32_100000");
            assert_eq!(
                NonFungibleId::from_u32(u32::MAX).to_canonical_string(),
                format!("u32_{}", u32::MAX)
            );

            // u64
            assert_eq!(NonFungibleId::from_u64(0).to_canonical_string(), "u64_0");
            assert_eq!(NonFungibleId::from_u64(1).to_canonical_string(), "u64_1");
            assert_eq!(NonFungibleId::from_u64(10).to_canonical_string(), "u64_10");
            assert_eq!(NonFungibleId::from_u64(100).to_canonical_string(), "u64_100");
            assert_eq!(
                NonFungibleId::from_u64(u64::MAX).to_canonical_string(),
                format!("u64_{}", u64::MAX)
            );

            // uuid
            assert_eq!(
                NonFungibleId::from_u256(
                    Hash32::from_hex("736bab0c3af393a0423c578ddcf7e19b81086f6ecbbc148713e95da75ef8171d")
                        .unwrap()
                        .into_array()
                )
                .to_canonical_string(),
                "uuid_736bab0c3af393a0423c578ddcf7e19b81086f6ecbbc148713e95da75ef8171d"
            );

            // string
            assert_eq!(
                NonFungibleId::try_from_string("hello_world")
                    .unwrap()
                    .to_canonical_string(),
                "str_hello_world"
            );
        }

        #[test]
        fn it_parses_a_display_string() {
            let id = NonFungibleId::from_u32(123);
            let s = id.to_string();
            let id2 = NonFungibleId::try_from_canonical_string(&s).unwrap();
            assert_eq!(id, id2);
        }
    }

    #[cfg(feature = "serde")]
    mod serde_deser {
        use super::*;

        #[test]
        fn string_serialization_and_deserialization() {
            let resx_str = "resource_0000000000000000000000000000000000000000000000000000000000000000";
            let resource = ResourceAddress::from_str(resx_str).unwrap();
            let v = NonFungibleAddress::new(resource, NonFungibleId::try_from_string("hello").unwrap());
            let json = serde_json::to_string_pretty(&v).unwrap();
            assert!(json.contains(resx_str));

            // Deserialize from JSON
            let r = serde_json::from_str::<NonFungibleAddress>(&json).unwrap();
            assert_eq!(r, v);

            // Check that CBOR does not include the string
            let cbor = tari_bor::encode(&v).unwrap();
            assert!(
                !cbor.windows(resx_str.len()).any(|window| window == resx_str.as_bytes()),
                "CBOR is serializing a string"
            );

            // Deserialize from CBOR
            let r = tari_bor::decode::<NonFungibleAddress>(&cbor).unwrap();
            assert_eq!(r, v);
        }
    }

    mod non_fungible_address_string {
        use super::*;

        #[test]
        fn it_parses_valid_strings() {
            NonFungibleAddress::from_str(
                "nft_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_str_SpecialNft",
            )
            .unwrap();
            NonFungibleAddress::from_str(
                "nft_a7cf4fd18ada7f367b1c102a9c158abc3754491665033231c5eb907fffffffff_uuid_7f19c3fe5fa13ff66a0d379fe5f9e3508acbd338db6bedd7350d8d565b2c5d32",
            )
            .unwrap();
        }

        #[test]
        fn it_rejects_invalid_strings() {
            NonFungibleAddress::from_str(
                "resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_nft_xxxxx_SpecialNft",
            )
            .unwrap_err();
            NonFungibleAddress::from_str(
                "nft_uuid_7f19c3fe5fa13ff66a0d379fe5f9e3508acbd338db6bedd7350d8d565b2c5d32ffffffff",
            )
            .unwrap_err();
            NonFungibleAddress::from_str("resource_x nft_str:SpecialNft").unwrap_err();
            NonFungibleAddress::from_str(
                "resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_nft_str_",
            )
            .unwrap_err();
            NonFungibleAddress::from_str(
                "resource_7cbfe29101c24924b1b6ccefbfff98986d648622272ae24f7585dab5ffffffff_nftx_str_SpecialNft",
            )
            .unwrap_err();
        }
    }
}
