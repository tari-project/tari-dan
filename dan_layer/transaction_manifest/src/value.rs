//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

use std::str::FromStr;

use proc_macro2::LexError;
use syn::{parse2, Lit};
use tari_engine_types::substate::SubstateAddress;

#[derive(Debug, Clone)]
pub enum ManifestValue {
    Address(SubstateAddress),
    Literal(Lit),
}

impl ManifestValue {
    pub fn address(&self) -> Option<SubstateAddress> {
        match self {
            Self::Address(addr) => Some(*addr),
            _ => None,
        }
    }
}

impl<T: Into<SubstateAddress>> From<T> for ManifestValue {
    fn from(addr: T) -> Self {
        ManifestValue::Address(addr.into())
    }
}

impl FromStr for ManifestValue {
    type Err = ManifestParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match SubstateAddress::from_str(s) {
            Ok(addr) => Ok(ManifestValue::Address(addr)),
            Err(_) => {
                let tokens = s.parse()?;
                let lit = parse2(tokens)?;
                Ok(ManifestValue::Literal(lit))
            },
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestParseError {
    #[error("Invalid address format: {0}")]
    AddressFormat(String),
    #[error("Invalid constant: {0}")]
    Constant(String),
    #[error("Invalid tokens: {0}")]
    Tokens(String),
}

// syn::Error and LexError use Rc's which are not Sync or Send
impl From<syn::Error> for ManifestParseError {
    fn from(e: syn::Error) -> Self {
        Self::Constant(e.to_string())
    }
}

impl From<LexError> for ManifestParseError {
    fn from(e: LexError) -> Self {
        Self::Tokens(e.to_string())
    }
}

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
            addr.address().unwrap(),
            SubstateAddress::Component(
                ComponentAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );

        let addr = "resource_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            addr.address().unwrap(),
            SubstateAddress::Resource(
                ResourceAddress::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );

        let addr = "vault_0000000000000000000000000000000000000000000000000000000000000000"
            .parse::<ManifestValue>()
            .unwrap();
        assert_eq!(
            addr.address().unwrap(),
            SubstateAddress::Vault(
                VaultId::from_hex("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
            )
        );
    }
}
