//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use async_trait::async_trait;
use indexmap::IndexMap;
use tari_dan_common_types::Epoch;
use tari_engine_types::{
    substate::{Substate, SubstateId},
    virtual_substate::VirtualSubstates,
};
use tari_transaction::{SubstateRequirement, Transaction};

pub struct ResolvedSubstates {
    pub local: IndexMap<SubstateId, Substate>,
    pub unresolved_foreign: HashSet<SubstateRequirement>,
}

#[async_trait]
pub trait SubstateResolver {
    type Error: Send + Sync + 'static;

    fn try_resolve_local(&self, transaction: &Transaction) -> Result<ResolvedSubstates, Self::Error>;

    async fn try_resolve_foreign(
        &self,
        requested_substates: &HashSet<SubstateRequirement>,
    ) -> Result<IndexMap<SubstateId, Substate>, Self::Error>;

    async fn resolve_virtual_substates(
        &self,
        transaction: &Transaction,
        current_epoch: Epoch,
    ) -> Result<VirtualSubstates, Self::Error>;
}
