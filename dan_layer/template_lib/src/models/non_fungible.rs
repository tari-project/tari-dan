//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tari_bor::BorTag;
use tari_template_abi::{
    call_engine,
    rust::{fmt, fmt::Display, write},
    EngineOp,
};

use super::BinaryTag;
use crate::{
    args::{InvokeResult, NonFungibleAction, NonFungibleInvokeArg},
    constants::PUBLIC_IDENTITY_RESOURCE_ADDRESS,
    crypto::RistrettoPublicKeyBytes,
    models::ResourceAddress,
    prelude::ResourceManager,
    Hash,
};

const DELIM: char = ':';

#[derive(Debug, Clone, Ord, PartialOrd, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub enum NonFungibleId {
    U256([u8; 32]),
    String(String),
    Uint32(u32),
    Uint64(u64),
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

    pub fn from_string<T: Into<String>>(id: T) -> Self {
        Self::String(id.into())
    }

    pub fn try_from_string<T: Into<String>>(id: T) -> Result<Self, ParseNonFungibleIdError> {
        let id = id.into();
        validate_nft_id_str(&id)?;
        Ok(NonFungibleId::String(id))
    }

    /// A string in one of the following formats
    /// - uuid:736bab0c3af393a0423c578ddcf7e19b81086f6ecbbc148713e95da75ef8171d
    /// - str:my_special_nft_name
    /// - u32:1234
    /// - u64:1234
    pub fn to_canonical_string(&self) -> String {
        let type_name = self.type_name();
        let mut s = String::with_capacity(type_name.len() + 1 + self.str_repr_len());
        s.push_str(self.type_name());
        s.push(DELIM);

        match self {
            NonFungibleId::U256(uuid) => {
                Hash::from(*uuid)
                    .write_hex_fmt(&mut s)
                    .expect("Invariant violated: String write is infallible");
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
            NonFungibleId::U256(_) => 64,
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
        let (id_type, id) = s.split_once(':').ok_or(ParseNonFungibleIdError::InvalidFormat)?;
        match id_type {
            "uuid" => Ok(NonFungibleId::U256(
                Hash::from_hex(id)
                    .map_err(|_| ParseNonFungibleIdError::InvalidUuid)?
                    .into_array(),
            )),
            "str" => {
                validate_nft_id_str(id)?;
                Ok(NonFungibleId::String(id.to_string()))
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
}

fn validate_nft_id_str(s: &str) -> Result<(), ParseNonFungibleIdError> {
    if s.is_empty() || s.len() > 64 {
        return Err(ParseNonFungibleIdError::InvalidStringLength);
    }
    Ok(())
}

impl Display for NonFungibleId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NonFungibleId::U256(v) => write!(f, "uuid:{}", Hash::from(*v)),
            NonFungibleId::String(s) => write!(f, "str:{}", s),
            NonFungibleId::Uint32(v) => write!(f, "u32:{}", v),
            NonFungibleId::Uint64(v) => write!(f, "u64:{}", v),
        }
    }
}

const TAG: u64 = BinaryTag::NonFungibleAddress.as_u64();

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NonFungibleAddress(BorTag<NonFungibleAddressContents, TAG>);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct NonFungibleAddressContents {
    resource_address: ResourceAddress,
    id: NonFungibleId,
}

impl NonFungibleAddress {
    pub fn new(resource_address: ResourceAddress, id: NonFungibleId) -> Self {
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
            *PUBLIC_IDENTITY_RESOURCE_ADDRESS,
            NonFungibleId::U256(public_key.into_array()),
        )
    }

    pub fn to_public_key(&self) -> Option<RistrettoPublicKeyBytes> {
        if self.0.resource_address != *PUBLIC_IDENTITY_RESOURCE_ADDRESS {
            return None;
        }
        match self.id() {
            NonFungibleId::U256(bytes) => match RistrettoPublicKeyBytes::from_bytes(bytes) {
                Ok(public_key) => Some(public_key),
                Err(_) => None,
            },
            _ => None,
        }
    }
}

impl From<NonFungibleAddressContents> for NonFungibleAddress {
    fn from(contents: NonFungibleAddressContents) -> Self {
        Self(BorTag::new(contents))
    }
}

impl Display for NonFungibleAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} nft_{}", self.0.resource_address, self.0.id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NonFungible {
    address: NonFungibleAddress,
}

impl NonFungible {
    pub fn new(address: NonFungibleAddress) -> Self {
        Self { address }
    }

    pub fn get_data<T: DeserializeOwned>(&self) -> T {
        let resp: InvokeResult = call_engine(
            EngineOp::NonFungibleInvoke,
            &NonFungibleInvokeArg {
                address: self.address.clone(),
                action: NonFungibleAction::GetData,
                args: vec![],
            },
        );

        resp.decode().expect("[get_data] Failed to decode NonFungible data")
    }

    pub fn get_mutable_data<T: DeserializeOwned>(&self) -> T {
        let resp: InvokeResult = call_engine(
            EngineOp::NonFungibleInvoke,
            &NonFungibleInvokeArg {
                address: self.address.clone(),
                action: NonFungibleAction::GetMutableData,
                args: vec![],
            },
        );

        resp.decode()
            .expect("[get_mutable_data] Failed to decode raw NonFungible mutable data")
    }

    pub fn set_mutable_data<T: Serialize + ?Sized>(&mut self, data: &T) {
        ResourceManager::get(*self.address.resource_address())
            .update_non_fungible_data(self.address.id().clone(), data);
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
                NonFungibleId::try_from_string("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"),
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
            assert_eq!(NonFungibleId::from_u32(0).to_canonical_string(), "u32:0");
            assert_eq!(NonFungibleId::from_u32(100000).to_canonical_string(), "u32:100000");
            assert_eq!(
                NonFungibleId::from_u32(u32::MAX).to_canonical_string(),
                format!("u32:{}", u32::MAX)
            );

            // u64
            assert_eq!(NonFungibleId::from_u64(0).to_canonical_string(), "u64:0");
            assert_eq!(NonFungibleId::from_u64(1).to_canonical_string(), "u64:1");
            assert_eq!(NonFungibleId::from_u64(10).to_canonical_string(), "u64:10");
            assert_eq!(NonFungibleId::from_u64(100).to_canonical_string(), "u64:100");
            assert_eq!(
                NonFungibleId::from_u64(u64::MAX).to_canonical_string(),
                format!("u64:{}", u64::MAX)
            );

            // uuid
            assert_eq!(
                NonFungibleId::from_u256(
                    Hash::from_hex("736bab0c3af393a0423c578ddcf7e19b81086f6ecbbc148713e95da75ef8171d")
                        .unwrap()
                        .into_array()
                )
                .to_canonical_string(),
                "uuid:736bab0c3af393a0423c578ddcf7e19b81086f6ecbbc148713e95da75ef8171d"
            );

            // string
            assert_eq!(
                NonFungibleId::try_from_string("hello_world")
                    .unwrap()
                    .to_canonical_string(),
                "str:hello_world"
            );
        }
    }
}
