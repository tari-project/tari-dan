//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::hash::Hash;

use serde::{Deserialize, Serialize};
use tari_engine_types::substate::{SubstateId, SubstateValue};

use crate::{consensus_models::BlockId, StateStoreReadTransaction, StateStoreWriteTransaction, StorageError};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BurntUtxo {
    pub substate_id: SubstateId,
    pub substate_value: SubstateValue,
    pub proposed_in_block: Option<BlockId>,
    pub base_layer_block_height: u64,
}

impl BurntUtxo {
    pub fn new(substate_id: SubstateId, substate_value: SubstateValue, base_layer_block_height: u64) -> Self {
        Self {
            substate_id,
            substate_value,
            proposed_in_block: None,
            base_layer_block_height,
        }
    }

    pub fn to_atom(&self) -> MintConfidentialOutputAtom {
        MintConfidentialOutputAtom {
            substate_id: self.substate_id.clone(),
        }
    }
}

impl BurntUtxo {
    pub fn insert<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.burnt_utxos_insert(self)
    }

    pub fn set_proposed_in_block<TTx: StateStoreWriteTransaction>(
        tx: &mut TTx,
        substate_id: &SubstateId,
        proposed_in_block: &BlockId,
    ) -> Result<(), StorageError> {
        tx.burnt_utxos_set_proposed_block(substate_id, proposed_in_block)?;
        Ok(())
    }

    pub fn get_all_unproposed<TTx: StateStoreReadTransaction>(
        tx: &TTx,
        block_id: &BlockId,
        limit: usize,
    ) -> Result<Vec<BurntUtxo>, StorageError> {
        tx.burnt_utxos_get_all_unproposed(block_id, limit)
    }

    pub fn has_unproposed<TTx: StateStoreReadTransaction>(tx: &TTx) -> Result<bool, StorageError> {
        Ok(tx.burnt_utxos_count()? > 0)
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
#[cfg_attr(
    feature = "ts",
    derive(ts_rs::TS),
    ts(export, export_to = "../../bindings/src/types/")
)]
pub struct MintConfidentialOutputAtom {
    pub substate_id: SubstateId,
}

impl MintConfidentialOutputAtom {
    pub fn get<TTx: StateStoreReadTransaction>(&self, tx: &TTx) -> Result<BurntUtxo, StorageError> {
        tx.burnt_utxos_get(&self.substate_id)
    }

    pub fn delete<TTx: StateStoreWriteTransaction>(&self, tx: &mut TTx) -> Result<(), StorageError> {
        tx.burnt_utxos_delete(&self.substate_id)
    }
}
