//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_engine_types::{Utxo, UtxoOutput};
use tari_indexer_client::types::{UtxoBurnt, UtxoSpent, UtxoUnspent, WalletUtxoUpdate};
use tari_ootle_common_types::StateVersion;
use tari_ootle_storage::{StorageError, time::PrimitiveDateTime};
use tari_template_lib_types::{ResourceAddress, UtxoAddress, UtxoId, crypto::PedersenCommitmentBytes};

use crate::storage_sqlite::{schema::utxos, serialization::deserialize_bincode};

#[derive(AsChangeset, Default)]
#[diesel(table_name = utxos)]
pub(crate) struct UtxoRecordUpdate {
    pub epoch: Option<i64>,
    pub version: Option<i64>,
    pub output: Option<Option<Vec<u8>>>,
    pub state_version: Option<i64>,
    pub is_spent: Option<bool>,
    pub is_burnt: Option<bool>,
    pub is_frozen: Option<bool>,
}

#[derive(Insertable)]
#[diesel(table_name = utxos)]
pub(crate) struct UtxoRecordInsert {
    pub commitment: String,
    pub public_nonce: String,
    pub version: i64,
    pub shard: i32,
    pub resource_address: String,
    pub state_version: i64,
    pub output: Option<Vec<u8>>,
    pub utxo_tag: i32,
    pub epoch: i64,
    pub is_spent: bool,
    pub is_burnt: bool,
    pub is_frozen: bool,
}
#[derive(Queryable)]
#[diesel(table_name = utxos)]
pub(crate) struct UtxoRecord {
    pub _id: i32,
    pub commitment: String,
    pub _public_nonce: String,
    pub version: i64,
    pub resource_address: String,
    pub _shard: i32,
    pub state_version: i64,
    pub output: Option<Vec<u8>>,
    pub _utxo_tag: i32,
    pub epoch: i64,
    pub _is_spent: bool,
    pub is_burnt: bool,
    pub is_frozen: bool,
    pub _created_at: PrimitiveDateTime,
}

impl UtxoRecord {
    pub fn try_convert_to_update(self) -> Result<(StateVersion, WalletUtxoUpdate), StorageError> {
        let id = self.to_utxo_id()?;
        match self.output {
            None => {
                // Spent or burnt
                if self.is_burnt {
                    Ok((
                        StateVersion::new(self.state_version as u64),
                        WalletUtxoUpdate::Burnt(UtxoBurnt {
                            id,
                            version: self.version as u64,
                        }),
                    ))
                } else {
                    Ok((
                        StateVersion::new(self.state_version as u64),
                        WalletUtxoUpdate::Spent(UtxoSpent {
                            id,
                            version: self.version as u64,
                        }),
                    ))
                }
            },
            Some(ref output) => {
                let output = deserialize_bincode::<UtxoOutput, _>(output).map_err(|e| StorageError::DecodingError {
                    operation: "UtxoRecord::try_convert",
                    item: "Utxo",
                    details: format!("Failed to parse Utxo from string: {}", e),
                })?;

                Ok((
                    StateVersion::new(self.state_version as u64),
                    WalletUtxoUpdate::Unspent(UtxoUnspent {
                        public_nonce: output.output.public_nonce,
                        tag: output.tag,
                    }),
                ))
            },
        }
    }

    pub fn try_convert_to_utxo(self) -> Result<(UtxoAddress, Utxo), StorageError> {
        let address = self.to_address()?;
        let utxo = Utxo {
            output: self.output.as_ref().map(deserialize_bincode).transpose().map_err(|e| {
                StorageError::DecodingError {
                    operation: "UtxoRecord::try_convert",
                    item: "Utxo",
                    details: format!("Failed to parse Utxo from string: {}", e),
                }
            })?,
            is_frozen: self.is_frozen,
        };

        Ok((address, utxo))
    }

    fn to_utxo_id(&self) -> Result<UtxoId, StorageError> {
        let commitment =
            PedersenCommitmentBytes::from_hex(&self.commitment).map_err(|e| StorageError::DecodingError {
                operation: "UtxoRecord::to_address",
                item: "UtxoAddress",
                details: format!("Failed to parse Commitment from string: {}", e),
            })?;
        Ok(commitment.into())
    }

    fn to_address(&self) -> Result<UtxoAddress, StorageError> {
        let resource_address =
            ResourceAddress::from_str(&self.resource_address).map_err(|e| StorageError::DecodingError {
                operation: "UtxoRecord::to_address",
                item: "UtxoAddress",
                details: format!("Failed to parse ResourceAddress from string: {}", e),
            })?;
        let id = self.to_utxo_id()?;
        Ok(UtxoAddress::new(resource_address, id))
    }
}
