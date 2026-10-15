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

use borsh::BorshSerialize;
use minicbor::{CborLen, Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{MaxSizeBytes, ValidatorSetHash};

const MAX_DATA_SIZE: usize = 256;
type ExtraFieldValue = MaxSizeBytes<MAX_DATA_SIZE>;

#[repr(u8)]
#[derive(
    Clone, Debug, Ord, PartialOrd, Eq, PartialEq, Serialize, Deserialize, BorshSerialize, Encode, Decode, CborLen,
)]
#[borsh(use_discriminant = true)]
pub enum ExtraFieldKey {
    #[n(0)]
    SidechainId = 0x00,
    /// The exhaust burn rate in force for the block's epoch, big-endian basis points. Present on
    /// every block, carried forward from the epoch's genesis block so that a node that joins by
    /// state sync reads the rate off a header rather than having to have witnessed the epoch open.
    #[n(1)]
    ExhaustBurnRate = 0x01,
    /// The exhaust burn rate the next epoch opens at, big-endian basis points. Present on an
    /// end-of-epoch block only: it is the value the quorum committing that block ratifies, and the
    /// next epoch's genesis block carries it into [`Self::ExhaustBurnRate`].
    #[n(2)]
    NextEpochExhaustBurnRate = 0x02,
    /// The [`ValidatorSetHash`] of the next epoch's committees, 32 bytes. Present on an end-of-epoch
    /// block only: the quorum committing that block ratifies it, which makes each end-of-epoch block
    /// a signed handover from one epoch's committees to the next.
    #[n(3)]
    NextEpochValidatorSetHash = 0x03,
    #[n(255)]
    Custom = 0xff,
}

#[derive(Clone, Debug, Deserialize, Serialize, Default, BorshSerialize, Encode, Decode, CborLen)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS), ts(export))]
#[cbor(transparent)]
pub struct ExtraData(
    #[n(0)]
    #[cfg_attr(feature = "ts", ts(type = "Record<string, any>"))]
    BTreeMap<ExtraFieldKey, ExtraFieldValue>,
);

impl ExtraData {
    pub const fn new() -> Self {
        Self(BTreeMap::new())
    }

    pub fn insert(&mut self, key: ExtraFieldKey, value: ExtraFieldValue) -> &mut Self {
        self.0.insert(key, value);
        self
    }

    pub fn get(&self, key: &ExtraFieldKey) -> Option<&ExtraFieldValue> {
        self.0.get(key)
    }

    pub fn contains_key(&self, key: &ExtraFieldKey) -> bool {
        self.0.contains_key(key)
    }

    /// Records a basis-point rate under `key` as two big-endian bytes.
    pub fn insert_bps(&mut self, key: ExtraFieldKey, bps: u16) -> &mut Self {
        self.insert(
            key,
            bps.to_be_bytes()
                .as_slice()
                .try_into()
                .expect("two bytes is within MAX_DATA_SIZE"),
        )
    }

    /// The basis-point rate under `key`, or `None` if the key is absent or does not hold exactly the
    /// two bytes [`Self::insert_bps`] writes.
    ///
    /// A malformed value reads as absent rather than as an error: this decodes a field of a block
    /// header that an untrusted peer may have built, and every caller's answer to "not the value I
    /// expect" is the same as its answer to "not there".
    pub fn get_bps(&self, key: &ExtraFieldKey) -> Option<u16> {
        let bytes: [u8; 2] = self.get(key)?.as_ref().try_into().ok()?;
        Some(u16::from_be_bytes(bytes))
    }

    /// Records a validator set hash under `key` as its 32 bytes.
    pub fn insert_validator_set_hash(&mut self, key: ExtraFieldKey, hash: &ValidatorSetHash) -> &mut Self {
        self.insert(
            key,
            hash.as_bytes().try_into().expect("32 bytes is within MAX_DATA_SIZE"),
        )
    }

    /// The validator set hash under `key`, or `None` if the key is absent or does not hold exactly 32 bytes.
    /// A malformed value reads as absent for the reason [`Self::get_bps`] gives.
    pub fn get_validator_set_hash(&self, key: &ExtraFieldKey) -> Option<ValidatorSetHash> {
        let bytes: [u8; 32] = self.get(key)?.as_ref().try_into().ok()?;
        Some(ValidatorSetHash::from_array(bytes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rate_round_trips_through_two_big_endian_bytes() {
        for bps in [0u16, 1, 500, 10_000, u16::MAX] {
            let mut extra_data = ExtraData::new();
            extra_data.insert_bps(ExtraFieldKey::ExhaustBurnRate, bps);
            assert_eq!(
                extra_data.get(&ExtraFieldKey::ExhaustBurnRate).unwrap().as_ref(),
                &bps.to_be_bytes()
            );
            assert_eq!(extra_data.get_bps(&ExtraFieldKey::ExhaustBurnRate), Some(bps));
        }
    }

    #[test]
    fn a_validator_set_hash_round_trips_through_its_bytes() {
        let hash = ValidatorSetHash::from_array([7; 32]);
        let mut extra_data = ExtraData::new();
        extra_data.insert_validator_set_hash(ExtraFieldKey::NextEpochValidatorSetHash, &hash);
        assert_eq!(
            extra_data
                .get(&ExtraFieldKey::NextEpochValidatorSetHash)
                .unwrap()
                .as_ref(),
            &[7; 32]
        );
        assert_eq!(
            extra_data.get_validator_set_hash(&ExtraFieldKey::NextEpochValidatorSetHash),
            Some(hash)
        );
    }

    #[test]
    fn a_validator_set_hash_that_is_not_32_bytes_reads_as_none() {
        for bytes in [[].as_slice(), [1; 31].as_slice(), [1; 33].as_slice()] {
            let mut extra_data = ExtraData::new();
            extra_data.insert(ExtraFieldKey::NextEpochValidatorSetHash, bytes.try_into().unwrap());
            assert_eq!(
                extra_data.get_validator_set_hash(&ExtraFieldKey::NextEpochValidatorSetHash),
                None
            );
        }
    }

    #[test]
    fn an_absent_key_reads_as_none() {
        assert_eq!(ExtraData::new().get_bps(&ExtraFieldKey::ExhaustBurnRate), None);
    }

    #[test]
    fn a_value_that_is_not_two_bytes_reads_as_none() {
        for bytes in [[].as_slice(), [1].as_slice(), [1, 2, 3].as_slice()] {
            let mut extra_data = ExtraData::new();
            extra_data.insert(ExtraFieldKey::ExhaustBurnRate, bytes.try_into().unwrap());
            assert_eq!(extra_data.get_bps(&ExtraFieldKey::ExhaustBurnRate), None);
        }
    }
}
