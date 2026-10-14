//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use tari_ootle_common_types::ValidatorSetHash;
use tari_ootle_storage::global::EpochData;

use crate::error::SqliteStorageError;

#[derive(Queryable)]
#[diesel(table_name = epochs)]
pub struct DbEpochData {
    pub epoch: i64,
    pub epoch_hash: Vec<u8>,
    pub validator_set_hash: Option<Vec<u8>>,
}

impl TryFrom<DbEpochData> for EpochData {
    type Error = SqliteStorageError;

    fn try_from(e: DbEpochData) -> Result<Self, Self::Error> {
        Ok(Self {
            epoch: e.epoch as u64,
            epoch_hash: e.epoch_hash.try_into().map_err(|e| SqliteStorageError::DecodingError {
                operation: "TryFrom<DbEpochData> for EpochData",
                item: "Epoch",
                details: format!("epoch_hash incorrect length: {e}"),
            })?,
            validator_set_hash: e
                .validator_set_hash
                .map(|hash| {
                    <[u8; 32]>::try_from(hash.as_slice())
                        .map(ValidatorSetHash::from_array)
                        .map_err(|_| SqliteStorageError::DecodingError {
                            operation: "TryFrom<DbEpochData> for EpochData",
                            item: "Epoch",
                            details: format!("validator_set_hash has {} bytes, expected 32", hash.len()),
                        })
                })
                .transpose()?,
        })
    }
}
