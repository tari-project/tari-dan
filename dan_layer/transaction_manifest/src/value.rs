//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Claus

use std::str::FromStr;

use proc_macro2::LexError;
use syn::{parse2, Lit};
use tari_engine_types::substate::SubstateAddress;
use tari_template_lib::models::{ComponentAddress, ResourceAddress, VaultId};

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
        match s.split_once('_') {
            Some(("component", addr)) => {
                let addr = ComponentAddress::from_hex(addr)
                    .map_err(|e| ManifestParseError::InvalidAddressFormat(e.to_string()))?;
                Ok(ManifestValue::Address(SubstateAddress::Component(addr)))
            },
            Some(("resource", addr)) => {
                let addr = ResourceAddress::from_hex(addr)
                    .map_err(|e| ManifestParseError::InvalidAddressFormat(e.to_string()))?;
                Ok(ManifestValue::Address(SubstateAddress::Resource(addr)))
            },
            Some(("vault", addr)) => {
                let id =
                    VaultId::from_hex(addr).map_err(|e| ManifestParseError::InvalidAddressFormat(e.to_string()))?;
                Ok(ManifestValue::Address(SubstateAddress::Vault(id)))
            },

            Some((_, _)) => Err(ManifestParseError::InvalidAddressFormat(s.to_string())),
            None => {
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
    InvalidAddressFormat(String),
    #[error("Invalid constant: {0}")]
    InvalidConstant(String),
    #[error("Invalid tokens: {0}")]
    InvalidTokens(String),
}

// syn::Error and LexError use Rc's which are not Sync or Send
impl From<syn::Error> for ManifestParseError {
    fn from(e: syn::Error) -> Self {
        Self::InvalidConstant(e.to_string())
    }
}

impl From<LexError> for ManifestParseError {
    fn from(e: LexError) -> Self {
        Self::InvalidTokens(e.to_string())
    }
}

#[cfg(test)]
mod tests {
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
