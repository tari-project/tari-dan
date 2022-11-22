//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Claus

use syn::Lit;
use tari_engine_types::substate::SubstateAddress;

#[derive(Debug, Clone)]
pub enum ManifestValue {
    Address(SubstateAddress),
    Constant(Lit),
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
