//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::str::FromStr;

use syn::{parse2, Lit};
use tari_bor::{BorError, Serialize};
use tari_engine_types::substate::SubstateId;
use tari_template_lib::{models::NonFungibleId, to_value};

#[derive(Debug, Clone)]
pub enum ManifestValue {
    SubstateId(SubstateId),
    Literal(Lit),
    NonFungibleId(NonFungibleId),
    Value(tari_bor::Value),
}

impl ManifestValue {
    pub fn new_value<T: Serialize>(value: &T) -> Result<Self, BorError> {
        Ok(Self::Value(to_value(value)?))
    }

    pub fn as_address(&self) -> Option<&SubstateId> {
        match self {
            Self::SubstateId(addr) => Some(addr),
            _ => None,
        }
    }
}

impl<T: Into<SubstateId>> From<T> for ManifestValue {
    fn from(addr: T) -> Self {
        ManifestValue::SubstateId(addr.into())
    }
}

// https://github.com/rust-lang/rfcs/issues/2758 :/
// impl From<NonFungibleId> for ManifestValue {
//     fn from(id: NonFungibleId) -> Self {
//         ManifestValue::NonFungibleId(id)
//     }
// }

impl FromStr for ManifestValue {
    type Err = ManifestParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        SubstateId::from_str(s)
            .ok()
            .map(ManifestValue::SubstateId)
            .or_else(|| {
                let id = NonFungibleId::try_from_canonical_string(s).ok()?;
                Some(ManifestValue::NonFungibleId(id))
            })
            .or_else(|| {
                let tokens = s.parse().ok()?;
                let lit = parse2(tokens).ok()?;
                Some(ManifestValue::Literal(lit))
            })
            .ok_or_else(|| ManifestParseError(s.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
#[error("Invalid manifest value '{0}'")]
pub struct ManifestParseError(String);

#[cfg(test)]
mod tests {
    use tari_template_lib::models::{ComponentAddress, ResourceAddress, VaultId};

    use super::*;

    #[test]
    fn it_parses_address_strings() {
        let addr = "component_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            *addr.as_address().unwrap(),
            SubstateId::Component(
                ComponentAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );

        let addr = "resource_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            *addr.as_address().unwrap(),
            SubstateId::Resource(
                ResourceAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );

        let addr = "vault_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            *addr.as_address().unwrap(),
            SubstateId::Vault(
                VaultId::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );
    }
}
