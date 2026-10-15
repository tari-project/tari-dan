// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Resolving the exhaust burn rate an epoch runs at.
//!
//! The rate reaches execution through the block header — `LockedEpoch` reads it off the block a
//! transaction is sequenced in — so it is resolved from state exactly twice per epoch: once by the
//! leader proposing the end-of-epoch block that opens the next epoch, and once by every replica
//! voting on that block. Between those two points it is a header field, never a substate read.
//!
//! Every node resolves the same value because the answer depends only on the epoch and on the
//! governance component, which lives on the global shard and so is held by every shard group.
//!
//! A node records the rate it opened each epoch at. Foreign proposals read that record rather than
//! the governance component, whose schedule only answers for the epochs a council call could still
//! affect: later calls prune entries that have been superseded.

use log::*;
use tari_engine_types::{
    fees::{ExhaustBurnRate, resolve_exhaust_burn_rate},
    substate::{SubstateId, SubstateValue},
};
use tari_ootle_common_types::{Epoch, optional::Optional};
use tari_ootle_storage::{StateStoreReadTransaction, consensus_models::SubstateRecord};
use tari_ootle_transaction::Network;
use tari_template_lib_types::{constants::BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS, governance::BurnRateGovernanceState};

use crate::hotstuff::HotStuffError;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::exhaust_burn_rate";

/// The exhaust burn rate `epoch` runs at.
///
/// Reads committed state rather than anything pending. A council transaction cannot name an
/// activation epoch less than `MIN_BURN_RATE_ACTIVATION_LEAD_EPOCHS` ahead of the epoch it executes
/// in: the template measures the lead time from the transaction's pinned epoch, and consensus aborts
/// a governance call executing in any other epoch. The end-of-epoch block of epoch `N` resolves the
/// rate for `N + 1`, so every change that can reach `N + 1` executed in `N - 1` at the latest.
///
/// Each shard group commits a change on its own schedule: a call that spans several shard groups
/// can finalize in a group after its pinned epoch. The groups agree on the rate for `N + 1` when
/// each has committed every such change before it proposes the end-of-epoch block of `N`.
pub fn resolve_epoch_exhaust_burn_rate<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    network: Network,
    epoch: Epoch,
) -> Result<ExhaustBurnRate, HotStuffError> {
    let governance = read_governance_rate(tx, epoch)?;
    Ok(resolve_exhaust_burn_rate(network, epoch, governance))
}

/// The exhaust burn rate a transaction pinned to a foreign proposal from `epoch` settles at.
///
/// This is the rate this node opened `epoch` at, which every shard group opened it at too. Only an
/// epoch this node has no record of is resolved from the governance component: one it has not
/// opened yet, which the lead time keeps every council change committed so far from reaching, or
/// one that opened before this node joined by state sync.
pub fn foreign_proposal_exhaust_burn_rate<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    network: Network,
    epoch: Epoch,
) -> Result<ExhaustBurnRate, HotStuffError> {
    if let Some(rate) = tx.epoch_exhaust_burn_rate_get(epoch)? {
        return Ok(rate);
    }
    resolve_epoch_exhaust_burn_rate(tx, network, epoch)
}

/// The rate the governance component holds for `epoch`.
///
/// `None` covers a council that has scheduled nothing reaching `epoch` and a component that is
/// absent, destroyed or holding something this binary cannot decode. All of them mean the same thing
/// to the caller — the table governs — and none of them may stop an epoch opening, which is why a
/// component that will not decode is logged and treated as silent rather than propagated.
fn read_governance_rate<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    epoch: Epoch,
) -> Result<Option<ExhaustBurnRate>, HotStuffError> {
    let substate_id = SubstateId::Component(BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS);
    let Some(record) = SubstateRecord::get_latest(tx, &substate_id).optional()? else {
        return Ok(None);
    };

    let Some(SubstateValue::Component(component)) = record.substate_value() else {
        return Ok(None);
    };

    let state: BurnRateGovernanceState = match tari_bor::from_value(component.state()) {
        Ok(state) => state,
        Err(err) => {
            warn!(
                target: LOG_TARGET,
                "🔥 Burn rate governance component did not decode ({err}). Resolving {epoch} from the rate schedule."
            );
            return Ok(None);
        },
    };

    Ok(state.rate_at(epoch.as_u64()).and_then(ExhaustBurnRate::try_new))
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{
        SubstateVersion,
        component::{Component, ComponentBody, ComponentHeader},
    };
    use tari_ootle_common_types::shard::Shard;
    use tari_ootle_storage::{
        StateStore,
        StateStoreWriteTransaction,
        StorageError,
        consensus_models::{SubstateTransition, SubstateUpdateBatch, SubstateValueOrHash},
    };
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
    use tari_template_lib_types::{
        SubstateOwnerRule,
        access_rules::ComponentAccessRules,
        governance::{BurnRateChange, prune_before, schedule_change},
    };

    use super::*;

    const NETWORK: Network = Network::LocalNet;

    fn change(activation_epoch: u64, rate_bps: u16) -> BurnRateChange {
        BurnRateChange {
            activation_epoch,
            rate_bps,
        }
    }

    fn commit_governance_state(
        store: &RocksDbStateStore<String>,
        schedule: Vec<BurnRateChange>,
        version: u64,
        epoch: Epoch,
    ) {
        let state = BurnRateGovernanceState {
            schedule,
            retired_from: None,
        };
        let component = Component {
            header: ComponentHeader {
                template_address: Default::default(),
                owner_rule: SubstateOwnerRule::None,
                access_rules: ComponentAccessRules::new(),
                entity_id: Default::default(),
            },
            body: ComponentBody::from_cbor_value(tari_bor::to_value(&state).unwrap()),
        };
        store
            .with_write_tx(|tx| {
                let mut batch = SubstateUpdateBatch::new(NETWORK, epoch);
                batch
                    .with_transition(Shard::from(0u32), version)
                    .push(SubstateTransition::Up {
                        id: SubstateId::Component(BURN_RATE_GOVERNANCE_COMPONENT_ADDRESS),
                        version: SubstateVersion::new(version),
                        substate_or_hash: SubstateValueOrHash::Value(Box::new(SubstateValue::Component(component))),
                    });
                SubstateRecord::commit_batch(tx, batch)?;
                Ok::<_, StorageError>(())
            })
            .unwrap();
    }

    /// Opens `epoch` the way a node does: at the rate resolved from committed state, recorded.
    fn open_epoch(store: &RocksDbStateStore<String>, epoch: Epoch) -> ExhaustBurnRate {
        store
            .with_write_tx(|tx| {
                let rate = resolve_epoch_exhaust_burn_rate(&**tx, NETWORK, epoch)?;
                tx.epoch_exhaust_burn_rate_set(epoch, rate)?;
                Ok::<_, HotStuffError>(rate)
            })
            .unwrap()
    }

    fn foreign_rate(store: &RocksDbStateStore<String>, epoch: Epoch) -> u16 {
        store
            .with_read_tx(|tx| foreign_proposal_exhaust_burn_rate(tx, NETWORK, epoch))
            .unwrap()
            .as_bps()
    }

    #[test]
    fn a_foreign_proposal_settles_at_the_rate_its_epoch_opened_with_after_a_prune() {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();

        let mut schedule = Vec::new();
        schedule_change(&mut schedule, change(12, 700));
        schedule_change(&mut schedule, change(13, 900));
        commit_governance_state(&store, schedule.clone(), 0, Epoch(11));

        assert_eq!(open_epoch(&store, Epoch(12)).as_bps(), 700);
        assert_eq!(open_epoch(&store, Epoch(13)).as_bps(), 900);
        assert_eq!(foreign_rate(&store, Epoch(12)), 700);

        // A council call executing in epoch 14 prunes the entry that governed epoch 12.
        prune_before(&mut schedule, 13);
        schedule_change(&mut schedule, change(16, 600));
        commit_governance_state(&store, schedule, 1, Epoch(14));

        assert_eq!(foreign_rate(&store, Epoch(12)), 700);
        assert_eq!(foreign_rate(&store, Epoch(13)), 900);
    }

    #[test]
    fn a_change_landing_on_an_opened_epoch_leaves_its_foreign_rate_alone() {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();

        commit_governance_state(&store, Vec::new(), 0, Epoch(5));
        let opened = open_epoch(&store, Epoch(7));

        commit_governance_state(&store, vec![change(7, 700)], 1, Epoch(7));

        assert_eq!(foreign_rate(&store, Epoch(7)), opened.as_bps());
    }

    #[test]
    fn an_epoch_not_yet_opened_resolves_from_the_schedule() {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();

        commit_governance_state(&store, vec![change(9, 700)], 0, Epoch(7));

        assert_eq!(foreign_rate(&store, Epoch(9)), 700);
    }
}
