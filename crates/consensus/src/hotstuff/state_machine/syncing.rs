//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::marker::PhantomData;

use log::*;
use tari_ootle_common_types::Epoch;

use crate::{
    hotstuff::{ConsensusWorkerContext, HotStuffError, state_machine::event::ConsensusStateEvent},
    traits::{ConsensusSpec, SyncManager},
};

const LOG_TARGET: &str = "tari::ootle::consensus::sm::syncing";

#[derive(Debug)]
pub struct Syncing<TSpec> {
    /// The epoch the caller proved is the highest finalised one — populated by the `NeedSync`
    /// event that triggered the transition into this state. `None` means use the default sync
    /// target (oracle's current epoch).
    target_epoch: Option<Epoch>,
    _spec: PhantomData<TSpec>,
}

impl<TSpec> Syncing<TSpec>
where
    TSpec: ConsensusSpec,
    HotStuffError: From<<TSpec::SyncManager as SyncManager>::Error>,
{
    pub(super) fn new(target_epoch: Option<Epoch>) -> Self {
        Self {
            target_epoch,
            _spec: PhantomData,
        }
    }

    pub(super) async fn on_enter(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
    ) -> Result<ConsensusStateEvent, HotStuffError> {
        // Read from the pre-sync state, which tells which epoch the pool was built in.
        let at_epoch_end = context.hotstuff.new_transactions_at_epoch_end()?;

        context.state_sync.sync(self.target_epoch).await?;

        // State sync only rewrites the state store, not the consensus transaction pool. Drop any pool
        // records so that on re-entering consensus we don't re-propose transactions the freshly-synced
        // state has already finalised (the rest of the committee removed them on commit, so they can
        // never gather a QC again - this wedges the network). A still-pending transaction is restored
        // to the pool from the record we kept, when a proposal for it arrives
        // (`OnReceiveNewTransaction::resequence_known_transactions`). The transactions this node held
        // unprepared at the end of the epoch it synced past are restored straight away: no proposal will
        // bring them back, because every member of the new shard group might have dropped them.
        let cleared = context.hotstuff.clear_transaction_pool(at_epoch_end).await?;
        if cleared.num_removed > 0 {
            info!(
                target: LOG_TARGET,
                "🧹 Cleared {} transaction(s) from the pool after state sync",
                cleared.num_removed
            );
        }
        if cleared.num_resequenced > 0 {
            info!(
                target: LOG_TARGET,
                "🎱 Sequenced {} transaction(s) held at the end of the previous epoch again",
                cleared.num_resequenced
            );
        }

        Ok(ConsensusStateEvent::SyncComplete)
    }
}
