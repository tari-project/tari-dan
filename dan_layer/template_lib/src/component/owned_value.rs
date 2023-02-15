//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, Decode, Encode};
use tari_template_abi::rust::{borrow::Borrow, collections::HashMap, format, io};

use crate::models::{Vault, VaultId};

#[derive(Debug, Clone)]
pub enum OwnedValue {
    Vault(VaultId),
    Sequence(Vec<OwnedValue>),
}

impl Encode for OwnedValue {
    fn serialize<W: io::Write>(&self, writer: &mut W) -> Result<(), io::Error> {
        let variant_idx: u8 = match self {
            OwnedValue::Vault(..) => 0u8,
            OwnedValue::Sequence(..) => 1u8,
        };
        writer.write_all(&variant_idx.to_le_bytes())?;
        match self {
            OwnedValue::Vault(id0) => {
                Encode::serialize(id0, writer)?;
            },
            OwnedValue::Sequence(id0) => {
                Encode::serialize(id0, writer)?;
            },
        }
        Ok(())
    }
}

impl Decode for OwnedValue {
    fn deserialize(buf: &mut &[u8]) -> Result<Self, io::Error> {
        let variant_idx: u8 = borsh::BorshDeserialize::deserialize(buf)?;
        let return_value = match variant_idx {
            0u8 => OwnedValue::Vault(Decode::deserialize(buf)?),
            1u8 => OwnedValue::Sequence(Decode::deserialize(buf)?),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("Unexpected variant index: {:?}", variant_idx),
                ));
            },
        };
        Ok(return_value)
    }
}

impl From<&Vault> for OwnedValue {
    fn from(vault: &Vault) -> Self {
        Self::Vault(*vault.borrow().vault_id())
    }
}

// impl<'a, K, V> From<&'a HashMap<K, V>> for OwnedValue
// where OwnedValue: From<&'a V>
// {
//     fn from(value: &'a HashMap<K, V>) -> Self {
//         Self::Sequence(value.values().map(|v| v.into()).collect())
//     }
// }
