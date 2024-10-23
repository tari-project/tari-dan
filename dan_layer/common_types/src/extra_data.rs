//   Copyright 2024. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{MaxSizeBytes, MaxSizeBytesError};

const MAX_DATA_SIZE: usize = 256;
type ExtraFieldValue = MaxSizeBytes<MAX_DATA_SIZE>;

#[repr(u8)]
#[derive(Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize)]
pub enum ExtraFieldKey {
    SidechainId = 0x00,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ExtraData(#[cfg_attr(feature = "ts", ts(type = "string"))] BTreeMap<ExtraFieldKey, ExtraFieldValue>);

impl ExtraData {
    pub const fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn insert<V: Into<ExtraFieldValue>>(&mut self, key: ExtraFieldKey, value: V) -> &mut Self {
        let value = value.into();
        self.0.insert(key, value);
        self
    }

    pub fn insert_sidechain_id(&mut self, sidechain_id: RistrettoPublicKey) -> Result<&mut Self, MaxSizeBytesError> {
        self.0
            .insert(ExtraFieldKey::SidechainId, sidechain_id.as_bytes().to_vec().try_into()?);
        Ok(self)
    }

    pub fn get(&self, key: &ExtraFieldKey) -> Option<&ExtraFieldValue> {
        self.0.get(key)
    }

    pub fn contains_key(&self, key: &ExtraFieldKey) -> bool {
        self.0.contains_key(key)
    }
}
