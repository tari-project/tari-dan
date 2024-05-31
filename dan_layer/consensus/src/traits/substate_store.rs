//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::SubstateAddress;
use tari_dan_storage::{
    consensus_models::{SubstateChange, SubstateRecord},
    StateStoreReadTransaction,
    StorageError,
};
use tari_engine_types::substate::{Substate, SubstateDiff};
use tari_transaction::{TransactionId, VersionedSubstateId};

pub trait ReadableSubstateStore {
    type Error;

    fn get(&self, key: &SubstateAddress) -> Result<Substate, Self::Error>;
}

pub trait WriteableSubstateStore: ReadableSubstateStore {
    fn put(&mut self, change: SubstateChange) -> Result<(), Self::Error>;

    fn put_diff(&mut self, transaction_id: TransactionId, diff: &SubstateDiff) -> Result<(), Self::Error> {
        for (id, version) in diff.down_iter() {
            self.put(SubstateChange::Down {
                id: VersionedSubstateId::new(id.clone(), *version),
                transaction_id,
            })?;
        }

        for (id, substate) in diff.up_iter() {
            self.put(SubstateChange::Up {
                id: VersionedSubstateId::new(id.clone(), substate.version()),
                substate: substate.clone(),
                transaction_id,
            })?;
        }

        Ok(())
    }
}

pub trait SubstateStore: ReadableSubstateStore + WriteableSubstateStore {}

impl<T: ReadableSubstateStore + WriteableSubstateStore> SubstateStore for T {}

impl<T: StateStoreReadTransaction> ReadableSubstateStore for &T {
    type Error = StorageError;

    fn get(&self, key: &SubstateAddress) -> Result<Substate, Self::Error> {
        let substate = SubstateRecord::get(*self, key)?;
        Ok(substate.into_substate())
    }
}
