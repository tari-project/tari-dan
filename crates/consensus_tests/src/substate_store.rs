//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{
    hotstuff::substate_store::{LockFailedError, PendingSubstateStore, SubstateStoreError},
    traits::{BlockTransactionExecutorError, CertificateStore, ReadableSubstateStore, WriteableSubstateStore},
};
use tari_consensus_types::{BlockId, LeafBlock};
use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    SubstateLockType,
    VersionedSubstateId,
    optional::IsNotFoundError,
    shard::Shard,
};
use tari_ootle_p2p::PeerAddress;
use tari_ootle_storage::{
    StateStore,
    consensus_models::{
        Block,
        RequireLockIntentRef,
        SubstateChange,
        SubstateRecord,
        SubstateTransition,
        SubstateUpdateBatch,
    },
};
use tari_ootle_transaction::Network;
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_template_lib_types::{ComponentAddress, EntityId, ObjectKey, SubstateOwnerRule};
use tempfile::TempDir;

use crate::support::{TEST_NUM_PRESHARDS, logging::setup_logger};

type TestStore = RocksDbStateStore<PeerAddress>;

#[test]
fn it_allows_substate_up_for_v0() {
    let (store, _tmp) = create_store();

    let id = new_substate_id(0);
    let value = new_substate_value(0);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);
    // Cannot put version 1
    store
        .put(SubstateChange::Up {
            id: id.clone(),
            shard: Shard::first(),
            substate: Box::new(Substate::new(1, value.clone())),
        })
        .unwrap_err();

    store
        .put(SubstateChange::Up {
            id: id.clone(),
            shard: Shard::first(),
            substate: Box::new(Substate::new(0, value)),
        })
        .unwrap();

    let s = store.get_latest_change(&id).unwrap().into_up().unwrap();
    assert_substate_eq(s, new_substate(0, 0));
}

#[test]
fn it_allows_down_then_up() {
    setup_logger();
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, 0);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    let s = store.get_latest_change(id.substate_id()).unwrap().into_up().unwrap();
    assert_substate_eq(s, new_substate(0, 0));

    store
        .put(SubstateChange::Down {
            id: id.clone(),
            shard: Shard::first(),
        })
        .unwrap();

    let next_version = id.to_next_version();
    store
        .put(SubstateChange::Up {
            id: next_version.substate_id().clone(),
            shard: Shard::first(),
            substate: Box::new(new_substate(1, next_version.version())),
        })
        .unwrap();

    let s = store.get(id.to_next_version().as_versioned_ref()).unwrap();
    assert_substate_eq(s, new_substate(1, 1));
    let s = store.get_latest_change(id.substate_id()).unwrap().into_up().unwrap();
    assert_substate_eq(s, new_substate(1, 1));
}

#[test]
fn it_fails_if_previous_version_is_not_down() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, 0);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);
    let err = store
        .put(SubstateChange::Up {
            id: id.substate_id().clone(),
            shard: Shard::first(),
            substate: Box::new(new_substate(1, id.version() + 1)),
        })
        .unwrap_err();

    assert!(matches!(err, SubstateStoreError::ExpectedSubstateDown { .. }));
}

#[test]
fn it_disallows_more_than_one_write_lock_non_local_only() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, 0);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), 0, SubstateLockType::Read),
            true,
        )
        .unwrap();
    store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), 0, SubstateLockType::Read),
            true,
        )
        .unwrap();

    let lock = store.new_locks().get(id.substate_id()).unwrap();
    let n = lock.iter().filter(|l| l.is_read()).count();
    assert_eq!(n, 2);

    let err = store
        .try_lock(
            tx_id(3),
            &RequireLockIntentRef::new(id.substate_id(), 0, SubstateLockType::Write),
            false,
        )
        .unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::LockConflict { .. }
    ));
}

#[test]
fn it_allows_requesting_the_same_lock_within_one_transaction() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, 0);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), 0, SubstateLockType::Write),
            false,
        )
        .unwrap();
    // Another transaction cannot lock the same substate
    let err = store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), 0, SubstateLockType::Output),
            false,
        )
        .unwrap_err();
    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::LockConflict { .. }
    ));

    // The same transaction is able to lock as an output
    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), 0, SubstateLockType::Output),
            false,
        )
        .unwrap();

    let n = store.new_locks().get(id.substate_id()).unwrap().len();
    assert_eq!(n, 2);
}

#[test]
fn a_substate_with_no_live_version_is_a_skippable_lock_failure() {
    // A substate with no live version (e.g. surfaced by put_diff at propose time when an input version was
    // already spent) must be classified as a recoverable lock failure. The proposer relies on
    // ok_lock_failed() to skip such transactions instead of propagating the error, which would otherwise
    // crash consensus.
    let id = VersionedSubstateId::new(new_substate_id(0), 0);
    let err = SubstateStoreError::SubstateNotFound { id };
    assert!(matches!(
        err.ok_lock_failed(),
        Ok(LockFailedError::SubstateNotFound { .. })
    ));
}

#[test]
fn a_substate_with_no_live_version_is_not_fatal_to_input_resolution() {
    // prepare() treats anything that is not a not-found error as fatal and propagates it out of consensus.
    // Resolving an input whose version was already spent must stay on the recoverable side of that test so
    // the transaction aborts on its own.
    let id = VersionedSubstateId::new(new_substate_id(0), 0);
    assert!(SubstateStoreError::SubstateNotFound { id: id.clone() }.is_not_found_error());
    assert!(
        BlockTransactionExecutorError::SubstateStoreError(SubstateStoreError::SubstateNotFound { id })
            .is_not_found_error()
    );
}

fn add_substate(store: &TestStore, seed: u8, version: u64) -> VersionedSubstateId {
    let id = new_substate_id(seed);
    let value = new_substate_value(seed);
    let mut batch = SubstateUpdateBatch::new(Network::LocalNet, Epoch::zero());
    batch.with_transition(Shard::first(), 0).push(SubstateTransition::Up {
        id: id.clone(),
        version,
        substate_or_hash: value.into(),
    });

    store
        .with_write_tx(|tx| SubstateRecord::commit_batch(tx, batch))
        .unwrap();

    VersionedSubstateId::new(id, version)
}

fn create_store() -> (TestStore, TempDir) {
    let temp_dir = tempfile::tempdir().unwrap();
    let store = RocksDbStateStore::open(&temp_dir, DatabaseOptions::default().with_debugging_data(true)).unwrap();
    store
        .with_write_tx(|tx| {
            let zero = Block::zero_block(Network::LocalNet, NumPreshards::P256);
            zero.justify().save(tx)?;
            zero.insert(tx)
        })
        .unwrap();
    (store, temp_dir)
}

fn create_pending_store<'a, 'tx, TStore: StateStore>(
    tx: &'a TStore::ReadTransaction<'tx>,
) -> PendingSubstateStore<'a, TStore::ReadTransaction<'tx>> {
    PendingSubstateStore::new(
        tx,
        LeafBlock {
            block_id: BlockId::zero(),
            height: NodeHeight::zero(),
            epoch: Epoch::zero(),
            shard_group: ShardGroup::all_shards(TEST_NUM_PRESHARDS),
        },
        TEST_NUM_PRESHARDS,
    )
}

fn new_substate_id(seed: u8) -> SubstateId {
    ComponentAddress::from_array([seed; ObjectKey::LENGTH]).into()
}

fn new_substate(seed: u8, version: u64) -> Substate {
    Substate::new(version, new_substate_value(seed))
}

fn new_substate_value(seed: u8) -> SubstateValue {
    Component {
        header: ComponentHeader {
            template_address: Default::default(),
            owner_rule: SubstateOwnerRule::None,
            access_rules: Default::default(),
            entity_id: [seed; EntityId::LENGTH].into(),
        },
        body: ComponentBody {
            state: tari_bor::Value::Null,
        },
    }
    .into()
}

fn tx_id(seed: u8) -> tari_ootle_transaction::TransactionId {
    [seed; tari_ootle_transaction::TransactionId::byte_size()].into()
}

fn assert_substate_eq(a: Substate, b: Substate) {
    assert_eq!(a.version(), b.version());
    assert_eq!(
        a.substate_value().as_component().unwrap().entity_id(),
        b.substate_value().as_component().unwrap().entity_id()
    );
}
