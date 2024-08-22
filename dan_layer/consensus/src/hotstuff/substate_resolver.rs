//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use indexmap::IndexMap;
use log::*;
use tari_engine_types::substate::Substate;
use tari_transaction::{Transaction, VersionedSubstateId};

use crate::{
    hotstuff::{
        substate_store::{PendingSubstateStore, SubstateStoreError},
        HotStuffError,
    },
    traits::ReadableSubstateStore,
};

const LOG_TARGET: &str = "tari::dan::consensus::substate_resolver";

pub struct SubstateResolver<'a, TSubstateStore> {
    // fields omitted
    store: PendingSubstateStore<&'a TSubstateStore>,
}

impl<'a, TSubstateStore: ReadableSubstateStore<Error = SubstateStoreError>> SubstateResolver<'a, TSubstateStore> {
    fn resolve_substates(
        &self,
        transaction: &Transaction,
    ) -> Result<IndexMap<VersionedSubstateId, Substate>, HotStuffError> {
        let mut resolved_substates = IndexMap::with_capacity(transaction.num_unique_inputs());

        for input in transaction.all_inputs_iter() {
            match input.version() {
                Some(version) => {
                    let id = VersionedSubstateId::new(input.substate_id, version);
                    let substate = self.store.get(&id)?;
                    info!(target: LOG_TARGET, "Resolved substate: {id}");
                    resolved_substates.insert(id, substate);
                },
                None => {
                    let (id, substate) = self.resolve_local_substate(input.substate_id, store)?;
                    info!(target: LOG_TARGET, "Resolved unversioned substate: {id}");
                    resolved_substates.insert(id, substate);
                },
            }
        }
        // TODO: we assume local only transactions, we need to implement multi-shard transactions.
        //       Suggest once we have pledges for foreign substates, we add them to a temporary pledge store and use
        //       that to resolve inputs.
        Ok(resolved_substates)
    }

    fn resolve_local_substate(
        &self,
        id: SubstateId,
    ) -> Result<(VersionedSubstateId, Substate), BlockTransactionExecutorError> {
        let substate = self.store.get_latest(&id).optional()?.ok_or_else(|| {
            BlockTransactionExecutorError::UnableToResolveSubstateId {
                substate_id: id.clone(),
            }
        })?;

        Ok((VersionedSubstateId::new(id, substate.version()), substate))
    }
}
