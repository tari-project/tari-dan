//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_consensus::{
    hotstuff::substate_store::{InputPledge, LockFailedError, PendingSubstateStore, SubstateStoreError},
    traits::{BlockTransactionExecutorError, CertificateStore, ReadableSubstateStore, WriteableSubstateStore},
};
use tari_consensus_types::{BlockId, LeafBlock};
use tari_engine_types::{
    component::{Component, ComponentBody, ComponentHeader},
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
};
use tari_ootle_common_types::{
    Epoch,
    NodeHeight,
    NumPreshards,
    ShardGroup,
    SubstateLockType,
    SubstateVersion,
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
        VersionedSubstateIdLockIntent,
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
            substate: Box::new(Substate::new(SubstateVersion::new(1), value.clone())),
        })
        .unwrap_err();

    store
        .put(SubstateChange::Up {
            id: id.clone(),
            shard: Shard::first(),
            substate: Box::new(Substate::new(SubstateVersion::ZERO, value)),
        })
        .unwrap();

    let s = store.get_latest_change(&id).unwrap().into_up().unwrap();
    assert_substate_eq(s, new_substate(0, SubstateVersion::ZERO));
}

#[test]
fn it_allows_down_then_up() {
    setup_logger();
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    let s = store.get_latest_change(id.substate_id()).unwrap().into_up().unwrap();
    assert_substate_eq(s, new_substate(0, SubstateVersion::ZERO));

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
    assert_substate_eq(s, new_substate(1, SubstateVersion::new(1)));
    let s = store.get_latest_change(id.substate_id()).unwrap().into_up().unwrap();
    assert_substate_eq(s, new_substate(1, SubstateVersion::new(1)));
}

#[test]
fn it_fails_if_previous_version_is_not_down() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);
    let err = store
        .put(SubstateChange::Up {
            id: id.substate_id().clone(),
            shard: Shard::first(),
            substate: Box::new(new_substate(1, id.version().next())),
        })
        .unwrap_err();

    assert!(matches!(err, SubstateStoreError::ExpectedSubstateDown { .. }));
}

#[test]
fn it_disallows_more_than_one_write_lock_non_local_only() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Read),
            true,
        )
        .unwrap();
    store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Read),
            true,
        )
        .unwrap();

    let lock = store.new_locks().get(id.substate_id()).unwrap();
    let n = lock.iter().filter(|l| l.is_read()).count();
    assert_eq!(n, 2);

    let err = store
        .try_lock(
            tx_id(3),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            false,
        )
        .unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::LockConflict { .. }
    ));
}

#[test]
fn it_allows_an_input_and_an_output_lock_within_one_transaction() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            false,
        )
        .unwrap();
    // Another transaction cannot lock the same substate
    let err = store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::new(1), SubstateLockType::Output),
            false,
        )
        .unwrap_err();
    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::LockConflict { .. }
    ));

    // The same transaction is able to lock the version it will create as an output
    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::new(1), SubstateLockType::Output),
            false,
        )
        .unwrap();

    let n = store.new_locks().get(id.substate_id()).unwrap().len();
    assert_eq!(n, 2);
}

#[test]
fn an_output_lock_over_a_write_lock_requires_the_version_to_not_exist() {
    // A WRITE lock over v0 says nothing about which version the OUTPUT lock claims. Granting OUTPUT over a live
    // version would let the transaction UP a version that already exists.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            false,
        )
        .unwrap();

    let err = store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Output),
            false,
        )
        .unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::SubstateExists { .. }
    ));
}

#[test]
fn an_output_lock_over_a_read_lock_requires_the_version_to_not_exist() {
    // Local-only rules allow an OUTPUT lock alongside another transaction's READ lock. The claimed version must
    // still not exist, otherwise two transactions in one block UP the same version.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Read),
            true,
        )
        .unwrap();

    let err = store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Output),
            true,
        )
        .unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::SubstateExists { .. }
    ));

    // The version the transaction will actually create is still lockable
    store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::new(1), SubstateLockType::Output),
            true,
        )
        .unwrap();
}

#[test]
fn an_output_lock_for_a_live_version_is_a_hard_conflict() {
    // try_lock_all classifies SubstateExists as a hard conflict, so the transaction ABORTs rather than being deferred
    // to a later block: the version it claims is taken for good.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Read),
            true,
        )
        .unwrap();

    let lock_status = store
        .try_lock_all(
            tx_id(2),
            [RequireLockIntentRef::new(
                id.substate_id(),
                SubstateVersion::ZERO,
                SubstateLockType::Output,
            )],
            true,
        )
        .unwrap();

    assert!(lock_status.is_hard_conflict());
    assert!(!lock_status.is_deferrable_conflict());
    assert!(matches!(
        lock_status.hard_conflict().unwrap(),
        LockFailedError::SubstateExists { .. }
    ));
}

#[test]
fn an_unversioned_lock_conflict_is_deferrable() {
    // The proposer skips a transaction whose locks failed without a hard conflict, and the replica no-votes any
    // block that sequences one. Both sides read the same predicate, so it must hold for the contended case.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            false,
        )
        .unwrap();

    // An unversioned input requirement: the transaction may be proposed later at whatever version is live then
    let intent = VersionedSubstateIdLockIntent::write(id.clone(), false);
    let lock_status = store.try_lock_all(tx_id(2), [&intent], false).unwrap();

    assert!(lock_status.is_any_failed());
    assert!(!lock_status.is_hard_conflict());
    assert!(lock_status.is_deferrable_conflict());
}

#[test]
fn a_versioned_lock_conflict_is_not_deferrable() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            false,
        )
        .unwrap();

    let intent = VersionedSubstateIdLockIntent::write(id.clone(), true);
    let lock_status = store.try_lock_all(tx_id(2), [&intent], false).unwrap();

    assert!(lock_status.is_hard_conflict());
    assert!(!lock_status.is_deferrable_conflict());
}

#[test]
fn locking_without_failures_is_not_a_deferrable_conflict() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    let intent = VersionedSubstateIdLockIntent::write(id.clone(), true);
    let lock_status = store.try_lock_all(tx_id(1), [&intent], false).unwrap();

    assert!(!lock_status.is_any_failed());
    assert!(!lock_status.is_deferrable_conflict());
}

#[test]
fn checking_input_locks_reports_a_write_lock_held_by_another_transaction() {
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    // Local-only rules admit no input lock over a WRITE lock either
    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            true,
        )
        .unwrap();

    let unversioned = VersionedSubstateIdLockIntent::read(id.clone(), false);
    let lock_status = store.check_input_locks(&tx_id(2), [&unversioned]).unwrap();
    assert!(lock_status.is_deferrable_conflict());

    let versioned = VersionedSubstateIdLockIntent::read(id.clone(), true);
    let lock_status = store.check_input_locks(&tx_id(2), [&versioned]).unwrap();
    assert!(lock_status.is_hard_conflict());

    let lock_status = store.check_input_locks(&tx_id(1), [&unversioned]).unwrap();
    assert!(!lock_status.is_any_failed());

    // Checking locks nothing, so the lock is still the first transaction's
    let lock = store.get_latest_lock_by_id(id.substate_id()).unwrap().unwrap();
    assert_eq!(*lock.transaction_id(), tx_id(1));
}

#[test]
fn checking_input_locks_ignores_a_lock_an_input_may_share() {
    let (store, _tmp) = create_store();

    let read_locked = add_substate(&store, 0, SubstateVersion::ZERO);
    let output_locked = add_substate(&store, 1, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(read_locked.substate_id(), SubstateVersion::ZERO, SubstateLockType::Read),
            false,
        )
        .unwrap();
    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(
                output_locked.substate_id(),
                SubstateVersion::new(1),
                SubstateLockType::Output,
            ),
            true,
        )
        .unwrap();

    // An executed transaction may only read a write-declared input, and may lock under local-only rules
    let intents = [
        VersionedSubstateIdLockIntent::write(read_locked.clone(), false),
        VersionedSubstateIdLockIntent::write(output_locked.clone(), false),
    ];
    let lock_status = store.check_input_locks(&tx_id(2), &intents).unwrap();
    assert!(!lock_status.is_any_failed());
}

#[test]
fn a_substate_with_no_live_version_is_a_skippable_lock_failure() {
    // A substate with no live version (e.g. surfaced by put_diff at propose time when an input version was
    // already spent) must be classified as a recoverable lock failure. The proposer relies on
    // ok_lock_failed() to skip such transactions instead of propagating the error, which would otherwise
    // crash consensus.
    let id = VersionedSubstateId::new(new_substate_id(0), SubstateVersion::ZERO);
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
    let id = VersionedSubstateId::new(new_substate_id(0), SubstateVersion::ZERO);
    assert!(SubstateStoreError::SubstateNotFound { id: id.clone() }.is_not_found_error());
    assert!(
        BlockTransactionExecutorError::SubstateStoreError(SubstateStoreError::SubstateNotFound { id })
            .is_not_found_error()
    );
}

#[test]
fn an_output_lock_for_a_destroyed_version_is_rejected() {
    // An OUTPUT lock names the version the transaction will create. A version that an earlier command in the block
    // destroyed has existed, so claiming it would UP a version that was already spent.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store
        .try_lock(
            tx_id(1),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Write),
            true,
        )
        .unwrap();
    store.put_diff(&write_diff(&id)).unwrap();

    let err = store
        .try_lock(
            tx_id(2),
            &RequireLockIntentRef::new(id.substate_id(), SubstateVersion::ZERO, SubstateLockType::Output),
            true,
        )
        .unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::SubstateExists { .. }
    ));
}

#[test]
fn a_put_diff_over_a_spent_input_is_a_skippable_lock_failure() {
    // The proposer skips a transaction whose diff no longer applies and the replica no-votes any block that
    // sequences one. Both depend on this failure classifying as a lock failure rather than a fatal store error.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    store.put_diff(&write_diff(&id)).unwrap();

    let err = store.put_diff(&write_diff(&id)).unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap(),
        LockFailedError::SubstateNotFound { .. }
    ));
}

#[test]
fn a_put_diff_upping_over_a_live_version_is_fatal() {
    // Only lock failures are deferred. A diff that UPs a version whose predecessor is still live is a broken
    // execution result, and both the proposer and the replica must propagate it.
    let (store, _tmp) = create_store();

    let id = add_substate(&store, 0, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);

    let mut diff = SubstateDiff::new();
    diff.up(id.substate_id().clone(), new_substate(1, id.version().next()));

    let err = store.put_diff(&diff).unwrap_err();

    assert!(matches!(
        err.ok_lock_failed().unwrap_err(),
        SubstateStoreError::ExpectedSubstateDown { .. }
    ));
}

#[test]
fn an_input_pledged_by_two_commands_is_counted_once() {
    let (store, _tmp) = create_store();
    let a = add_substate(&store, 0, SubstateVersion::ZERO);
    let b = add_substate(&store, 1, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);
    let pledge = store.pledge_inputs([a.as_versioned_ref()], usize::MAX).unwrap();
    assert_eq!(pledge, InputPledge::Pledged);
    let one_input = store.pledge_bytes();
    assert!(one_input > 0);

    store
        .pledge_inputs([a.as_versioned_ref(), b.as_versioned_ref()], usize::MAX)
        .unwrap();
    // Both substates have values of the same size
    assert_eq!(store.pledge_bytes(), 2 * one_input);
}

#[test]
fn a_proposing_store_refuses_inputs_past_its_pledge_budget() {
    let (store, _tmp) = create_store();
    let a = add_substate(&store, 0, SubstateVersion::ZERO);
    let b = add_substate(&store, 1, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let one_input = {
        let mut store = create_pending_store::<TestStore>(&tx);
        store.pledge_inputs([a.as_versioned_ref()], usize::MAX).unwrap();
        store.pledge_bytes()
    };

    let mut store = create_pending_store::<TestStore>(&tx).with_pledge_budget(one_input);
    assert_eq!(
        store.pledge_inputs([a.as_versioned_ref()], usize::MAX).unwrap(),
        InputPledge::Pledged
    );
    assert_eq!(
        store.pledge_inputs([b.as_versioned_ref()], usize::MAX).unwrap(),
        InputPledge::BlockBudgetReached {
            total_bytes: 2 * one_input,
            max_bytes: one_input,
        }
    );
    // An input already in the block's pledge adds nothing to it
    assert_eq!(
        store.pledge_inputs([a.as_versioned_ref()], usize::MAX).unwrap(),
        InputPledge::Pledged
    );
    assert_eq!(store.pledge_bytes(), one_input);
}

#[test]
fn inputs_over_the_transaction_limit_are_not_pledged() {
    let (store, _tmp) = create_store();
    let a = add_substate(&store, 0, SubstateVersion::ZERO);
    let b = add_substate(&store, 1, SubstateVersion::ZERO);

    let tx = store.create_read_tx().unwrap();
    let mut store = create_pending_store::<TestStore>(&tx);
    store.pledge_inputs([a.as_versioned_ref()], usize::MAX).unwrap();
    let one_input = store.pledge_bytes();

    let mut store = create_pending_store::<TestStore>(&tx);
    assert_eq!(
        store
            .pledge_inputs([a.as_versioned_ref(), b.as_versioned_ref()], one_input)
            .unwrap(),
        InputPledge::TransactionTooLarge { bytes: 2 * one_input }
    );
    assert_eq!(store.pledge_bytes(), 0);
}

/// The diff of a transaction that takes `id` as a WRITE input: it spends the locked version and creates the next.
fn write_diff(id: &VersionedSubstateId) -> SubstateDiff {
    let mut diff = SubstateDiff::new();
    diff.down(id.substate_id().clone(), id.version());
    diff.up(id.substate_id().clone(), new_substate(1, id.version().next()));
    diff
}

fn add_substate(store: &TestStore, seed: u8, version: SubstateVersion) -> VersionedSubstateId {
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
    PendingSubstateStore::init(
        tx,
        LeafBlock {
            block_id: BlockId::zero(),
            height: NodeHeight::zero(),
            epoch: Epoch::zero(),
            shard_group: ShardGroup::all_shards(TEST_NUM_PRESHARDS),
        },
        TEST_NUM_PRESHARDS,
    )
    .unwrap()
}

fn new_substate_id(seed: u8) -> SubstateId {
    ComponentAddress::from_array([seed; ObjectKey::LENGTH]).into()
}

fn new_substate(seed: u8, version: SubstateVersion) -> Substate {
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
