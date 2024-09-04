//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde_with::serde_as;
use tari_template_abi::rust::{
    fmt,
    fmt::{Display, Formatter},
    ops::{Deref, DerefMut},
    str::FromStr,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct EntityId([u8; Self::LENGTH]);

impl EntityId {
    pub const LENGTH: usize = 20;

    pub const fn new(bytes: [u8; Self::LENGTH]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; Self::LENGTH] {
        &self.0
    }

    pub const fn from_array(bytes: [u8; Self::LENGTH]) -> Self {
        Self(bytes)
    }

    pub fn into_array(self) -> [u8; Self::LENGTH] {
        self.0
    }

    pub fn from_hex(s: &str) -> Result<Self, KeyParseError> {
        from_hex(s).map(Self::from_array)
    }

    pub fn write_hex_fmt<W: fmt::Write>(&self, writer: &mut W) -> fmt::Result {
        for b in self.0 {
            write!(writer, "{:02x?}", b)?;
        }
        Ok(())
    }
}

impl AsRef<[u8]> for EntityId {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<[u8; Self::LENGTH]> for EntityId {
    fn from(hash: [u8; Self::LENGTH]) -> Self {
        Self::from_array(hash)
    }
}

impl FromStr for EntityId {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::from_hex(s)
    }
}

impl TryFrom<&[u8]> for EntityId {
    type Error = KeyParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != Self::LENGTH {
            return Err(KeyParseError);
        }
        let mut hash = [0u8; Self::LENGTH];
        hash.copy_from_slice(value);
        Ok(Self::from_array(hash))
    }
}

impl TryFrom<Vec<u8>> for EntityId {
    type Error = KeyParseError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        Self::try_from(value.as_slice())
    }
}

impl Deref for EntityId {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for EntityId {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for EntityId {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        self.write_hex_fmt(f)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Default, serde::Serialize, serde::Deserialize)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct ComponentKey([u8; Self::LENGTH]);

impl ComponentKey {
    pub const LENGTH: usize = 12;

    pub const fn new(bytes: [u8; Self::LENGTH]) -> Self {
        Self(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; Self::LENGTH] {
        &self.0
    }
}

impl From<[u8; Self::LENGTH]> for ComponentKey {
    fn from(hash: [u8; Self::LENGTH]) -> Self {
        Self::new(hash)
    }
}

/// Representation of a 32-byte object key
#[serde_as]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Ord, PartialOrd, Hash, Default, serde::Serialize, serde::Deserialize)]
#[serde(transparent)]
pub struct ObjectKey(#[serde_as(as = "serde_with::Bytes")] [u8; Self::LENGTH]);

impl ObjectKey {
    pub const LENGTH: usize = EntityId::LENGTH + ComponentKey::LENGTH;

    pub fn new(entity_id: EntityId, component_key: ComponentKey) -> Self {
        let mut buf = [0u8; Self::LENGTH];
        buf[..EntityId::LENGTH].copy_from_slice(entity_id.as_bytes());
        buf[EntityId::LENGTH..].copy_from_slice(component_key.as_bytes());
        Self(buf)
    }

    pub const fn from_array(bytes: [u8; Self::LENGTH]) -> Self {
        Self(bytes)
    }

    pub fn into_array(self) -> [u8; Self::LENGTH] {
        self.0
    }

    pub fn from_hex(s: &str) -> Result<Self, KeyParseError> {
        from_hex(s).map(Self::from_array)
    }

    pub fn write_hex_fmt<W: fmt::Write>(&self, writer: &mut W) -> fmt::Result {
        for b in self.0 {
            write!(writer, "{:02x?}", b)?;
        }
        Ok(())
    }

    pub fn try_from_vec(data: Vec<u8>) -> Result<Self, KeyParseError> {
        Self::try_from(data.as_slice())
    }

    pub fn as_entity_id(&self) -> EntityId {
        let mut entity_id = [0u8; EntityId::LENGTH];
        entity_id.copy_from_slice(&self.0[..EntityId::LENGTH]);
        EntityId(entity_id)
    }

    pub fn as_component_key(&self) -> ComponentKey {
        let mut component_key = [0u8; ComponentKey::LENGTH];
        component_key.copy_from_slice(&self.0[EntityId::LENGTH..]);
        ComponentKey(component_key)
    }
}

impl AsRef<[u8]> for ObjectKey {
    fn as_ref(&self) -> &[u8] {
        self.0.as_ref()
    }
}

impl From<[u8; Self::LENGTH]> for ObjectKey {
    fn from(hash: [u8; Self::LENGTH]) -> Self {
        Self::from_array(hash)
    }
}

impl FromStr for ObjectKey {
    type Err = KeyParseError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        ObjectKey::from_hex(s)
    }
}

impl TryFrom<&[u8]> for ObjectKey {
    type Error = KeyParseError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        if value.len() != Self::LENGTH {
            return Err(KeyParseError);
        }
        let mut hash = [0u8; Self::LENGTH];
        hash.copy_from_slice(value);
        Ok(ObjectKey::from_array(hash))
    }
}

impl TryFrom<Vec<u8>> for ObjectKey {
    type Error = KeyParseError;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        ObjectKey::try_from(value.as_slice())
    }
}

impl Deref for ObjectKey {
    type Target = [u8];

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl DerefMut for ObjectKey {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl Display for ObjectKey {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        for x in self.0 {
            write!(f, "{:02x?}", x)?;
        }
        Ok(())
    }
}

/// Representation of a hash parsing error
#[derive(Debug)]
pub struct KeyParseError;

#[cfg(feature = "std")]
impl std::error::Error for KeyParseError {}

impl Display for KeyParseError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "Failed to parse byte key")
    }
}

pub fn from_hex<const N: usize>(s: &str) -> Result<[u8; N], KeyParseError> {
    if s.len() != N * 2 {
        return Err(KeyParseError);
    }

    let mut arr = [0u8; N];
    for (i, h) in arr.iter_mut().enumerate() {
        *h = u8::from_str_radix(&s[2 * i..2 * (i + 1)], 16).map_err(|_| KeyParseError)?;
    }
    Ok(arr)
}
