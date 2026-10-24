//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::str::FromStr;

use tari_engine_types::{Utxo, encoded::Encoded};
use tari_indexer_client::types::{UtxoBurnt, UtxoSpent, UtxoUnspent, WalletUtxoUpdate};
use tari_ootle_common_types::{StateVersion, SubstateVersion};
use tari_ootle_storage::{StorageError, time::PrimitiveDateTime};
use tari_template_lib_types::{
    ResourceAddress,
    UtxoAddress,
    UtxoId,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, UtxoTag},
};

use crate::storage_sqlite::{schema::utxos, serialization::deserialize_hex_try_from};

#[derive(AsChangeset, Default)]
#[diesel(table_name = utxos)]
pub(crate) struct UtxoRecordUpdate {
    pub epoch: Option<i64>,
    pub version: Option<i64>,
    pub utxo: Option<Option<Vec<u8>>>,
    pub state_version: Option<i64>,
    pub is_spent: Option<bool>,
    pub is_burnt: Option<bool>,
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
    pub utxo: Option<Vec<u8>>,
    pub utxo_tag: i32,
    pub epoch: i64,
    pub is_spent: bool,
    pub is_burnt: bool,
}
#[derive(Queryable)]
#[diesel(table_name = utxos)]
pub(crate) struct UtxoRecord {
    pub _id: i64,
    pub commitment: String,
    pub public_nonce: String,
    pub version: i64,
    pub resource_address: String,
    pub _shard: i32,
    pub state_version: i64,
    pub utxo: Option<Vec<u8>>,
    pub utxo_tag: i32,
    pub epoch: i64,
    pub is_spent: bool,
    pub is_burnt: bool,
    pub _created_at: PrimitiveDateTime,
}

impl UtxoRecord {
    pub fn try_convert_to_update(self) -> Result<(StateVersion, WalletUtxoUpdate), StorageError> {
        let id = self.to_utxo_id()?;
        match self.utxo {
            None => {
                // Spent or burnt
                if self.is_burnt {
                    Ok((
                        StateVersion::new(self.state_version as u64),
                        WalletUtxoUpdate::Burnt(UtxoBurnt {
                            id,
                            version: SubstateVersion::new(self.version as u64),
                        }),
                    ))
                } else {
                    Ok((
                        StateVersion::new(self.state_version as u64),
                        WalletUtxoUpdate::Spent(UtxoSpent {
                            id,
                            version: SubstateVersion::new(self.version as u64),
                        }),
                    ))
                }
            },
            Some(_) => {
                let public_nonce = deserialize_hex_try_from::<RistrettoPublicKeyBytes, _>(&self.public_nonce)?;
                let tag = UtxoTag::new(self.utxo_tag as u32);

                Ok((
                    StateVersion::new(self.state_version as u64),
                    WalletUtxoUpdate::Unspent(UtxoUnspent { public_nonce, tag }),
                ))
            },
        }
    }

    /// The address and stored encoding of an unspent UTXO.
    pub fn try_convert_to_utxo(self) -> Result<(UtxoAddress, Encoded<Utxo>), StorageError> {
        let address = self.to_address()?;
        let utxo = self.utxo.ok_or_else(|| StorageError::DataInconsistency {
            details: format!("Unspent UTXO {} has no stored value", self.commitment),
        })?;
        Ok((address, Encoded::from_bytes(utxo)))
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
