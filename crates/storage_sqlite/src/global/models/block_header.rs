//   Copyright 2022. The Tari Project
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

use tari_ootle_common_types::Epoch;
use tari_ootle_storage::{global, time};

use crate::{error::SqliteStorageError, global::schema::block_headers};

#[derive(Debug, Identifiable, Queryable)]
#[diesel(table_name = block_headers)]
pub struct BlockHeaderModel {
    pub id: i32,
    pub epoch: i64,
    pub height: i64,
    pub block_hash: Vec<u8>,
    pub block_output_merkle_root: Vec<u8>,
    pub validator_node_merkle_root: Vec<u8>,
    pub _created_at: time::PrimitiveDateTime,
}

impl TryFrom<BlockHeaderModel> for global::BlockHeaderModel {
    type Error = SqliteStorageError;

    fn try_from(value: BlockHeaderModel) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: Epoch(value.epoch as u64),
            height: value.height as u64,
            block_hash: value
                .block_hash
                .try_into()
                .map_err(|e| SqliteStorageError::ConversionError {
                    reason: format!("Block hash invalid: {e}"),
                })?,
            block_output_merkle_root: value.block_output_merkle_root.try_into().map_err(|e| {
                SqliteStorageError::ConversionError {
                    reason: format!("Block output merkle root invalid: {e}"),
                }
            })?,
            validator_node_merkle_root: value.validator_node_merkle_root.try_into().map_err(|e| {
                SqliteStorageError::ConversionError {
                    reason: format!("Validator node merkle root invalid: {e}"),
                }
            })?,
        })
    }
}

#[cfg(test)]
mod tests {
    use tari_sidechain::FixedHash;

    use super::*;

    fn model(block_output_merkle_root: Vec<u8>) -> BlockHeaderModel {
        BlockHeaderModel {
            id: 1,
            epoch: 3,
            height: 42,
            block_hash: vec![1u8; 32],
            block_output_merkle_root,
            validator_node_merkle_root: vec![3u8; 32],
            _created_at: time::PrimitiveDateTime::MIN,
        }
    }

    #[test]
    fn converts_to_domain_model() {
        let header = global::BlockHeaderModel::try_from(model(vec![4u8; 32])).unwrap();
        assert_eq!(header.epoch, Epoch(3));
        assert_eq!(header.height, 42);
        assert_eq!(header.block_hash, FixedHash::from([1u8; 32]));
        assert_eq!(header.block_output_merkle_root, FixedHash::from([4u8; 32]));
        assert_eq!(header.validator_node_merkle_root, FixedHash::from([3u8; 32]));
    }

    #[test]
    fn rejects_wrong_length_block_output_merkle_root() {
        for len in [0, 31, 33] {
            let err = global::BlockHeaderModel::try_from(model(vec![4u8; len])).unwrap_err();
            assert!(
                matches!(&err, SqliteStorageError::ConversionError { reason } if reason.starts_with("Block output merkle root invalid")),
                "len {len}: {err}"
            );
        }
    }
}
