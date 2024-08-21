//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_storage::{
    consensus_models::{SubstateChange, SubstateRecord},
    StateStoreReadTransaction,
};
use tari_engine_types::substate::{Substate, SubstateDiff};
use tari_transaction::{TransactionId, VersionedSubstateId};

use crate::hotstuff::substate_store::SubstateStoreError;

pub trait ReadableSubstateStore {
    type Error;

    fn get(&self, id: &VersionedSubstateId) -> Result<Substate, Self::Error>;
}

pub trait WriteableSubstateStore: ReadableSubstateStore {
    fn put(&mut self, change: SubstateChange) -> Result<(), Self::Error>;

    fn put_diff(&mut self, transaction_id: TransactionId, diff: &SubstateDiff) -> Result<(), Self::Error>;
}

pub trait SubstateStore: ReadableSubstateStore + WriteableSubstateStore {}

impl<T: ReadableSubstateStore + WriteableSubstateStore> SubstateStore for T {}

impl<T: StateStoreReadTransaction> ReadableSubstateStore for &T {
    type Error = SubstateStoreError;

    fn get(&self, id: &VersionedSubstateId) -> Result<Substate, Self::Error> {
        let substate = SubstateRecord::get(*self, &id.to_substate_address())?;
        Ok(substate.into_substate())
    }
}
