//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;
use std::time::Duration;

use helpers::{assert_eq_debug, commit_chain, create_chain, create_random_substate_id, create_rocksdb, create_tx_atom};
use tari_common_types::types::{FixedHash, PrivateKey};
use tari_consensus_types::{Decision, PcId, ShardGroupAccumulatedData};
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
    fees::FeeBreakdown,
    substate::SubstateDiff,
};
use tari_ootle_common_types::{Epoch, ExtraData, NodeHeight, ProtocolVersion, SubstateRequirement};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{
        Block,
        BlockTransactionExecution,
        BookkeepingModel,
        Command,
        Evidence,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
    },
};
use tari_ootle_transaction::{Instruction, Transaction};
use tari_template_lib::types::Hash32;
use tari_utilities::epoch_time::EpochTime;

mod confirm_all_transitions {
    use tari_ootle_transaction::Network;
    use tari_template_lib_types::crypto::SchnorrSignatureBytes;

    use super::*;
    use crate::helpers::num_preshards;

    #[test]
    fn it_sets_pending_stage_to_stage_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        it_sets_pending_stage_to_stage(db);
    }

    fn it_sets_pending_stage_to_stage(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        let atom1 = create_tx_atom();
        let atom2 = create_tx_atom();
        let atom3 = create_tx_atom();

        let network = Network::LocalNet;
        let zero_block = Block::zero_block(network, num_preshards());
        zero_block.insert(&mut tx).unwrap();
        tx.proposal_certificates_save(zero_block.justify()).unwrap();
        tx.blocks_set_qcs(zero_block.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();

        let shard_group = zero_block.shard_group();

        let block1 = Block::create(
            network,
            ProtocolVersion::V0,
            *zero_block.id(),
            zero_block.justify().clone(),
            None,
            NodeHeight(1),
            Epoch(0),
            shard_group,
            Default::default(),
            // Need to have a command in, otherwise this block will not be included internally in the query because it
            // cannot cause a state change without any commands
            [Command::LocalPrepare(atom1.clone())].into_iter().collect(),
            Default::default(),
            Default::default(),
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::default(),
        )
        .unwrap();
        block1.insert(&mut tx).unwrap();
        block1.as_locked().set(&mut tx).unwrap();
        block1.as_leaf().set(&mut tx).unwrap();

        tx.transaction_pool_insert_new(atom1.id, atom1.decision, &Evidence::empty(), true, false, Epoch(1), 0)
            .unwrap();
        tx.transaction_pool_insert_new(atom2.id, atom2.decision, &Evidence::empty(), true, false, Epoch(1), 0)
            .unwrap();
        tx.transaction_pool_insert_new(atom3.id, atom3.decision, &Evidence::empty(), true, false, Epoch(1), 0)
            .unwrap();
        let block_id = *block1.id();
        let transactions = tx.transaction_pool_get_all(1000).unwrap();
        let mut tx_1 = transactions.iter().find(|tx| *tx.id() == atom1.id).unwrap().clone();
        let mut tx_2 = transactions.iter().find(|tx| *tx.id() == atom2.id).unwrap().clone();
        let mut tx_3 = transactions.iter().find(|tx| *tx.id() == atom3.id).unwrap().clone();

        assert!(tx.transaction_pool_exists(&atom1.id).unwrap());
        assert!(tx.transaction_pool_exists(&atom2.id).unwrap());
        assert!(tx.transaction_pool_exists(&atom3.id).unwrap());

        tx_1.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, shard_group)
            .unwrap();
        tx_1.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, shard_group)
            .unwrap();
        tx_2.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, shard_group)
            .unwrap();
        tx_3.set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, shard_group)
            .unwrap();

        tx.transaction_pool_add_pending_update(&block1.as_leaf(), &TransactionPoolStatusUpdate::new(tx_1, true))
            .unwrap();
        tx.transaction_pool_add_pending_update(&block1.as_leaf(), &TransactionPoolStatusUpdate::new(tx_2, true))
            .unwrap();
        tx.transaction_pool_add_pending_update(&block1.as_leaf(), &TransactionPoolStatusUpdate::new(tx_3, true))
            .unwrap();

        let rec = tx.transaction_pool_get_many_ready(u64::MAX, 10, &block_id).unwrap();
        assert_eq!(rec.len(), 3);

        let rec = tx.transaction_pool_get_for_blocks(&block_id, &atom1.id).unwrap();
        assert!(rec.committed_stage().is_new());
        assert!(rec.pending_stage().unwrap().is_local_prepared());

        let rec = tx.transaction_pool_get_for_blocks(&block_id, &atom2.id).unwrap();
        assert!(rec.committed_stage().is_new());
        assert!(rec.pending_stage().unwrap().is_local_prepared());

        tx.transaction_pool_confirm_all_transitions(&block1.as_leaf()).unwrap();

        let rec = tx.transaction_pool_get_for_blocks(&block_id, &atom1.id).unwrap();
        assert!(rec.committed_stage().is_local_prepared());
        assert!(rec.pending_stage().is_none());

        let rec = tx.transaction_pool_get_for_blocks(&block_id, &atom2.id).unwrap();
        assert_eq!(rec.committed_stage(), TransactionPoolStage::LocalPrepared);
        assert_eq!(rec.pending_stage(), None);

        let rec = tx.transaction_pool_get_for_blocks(&block_id, &atom3.id).unwrap();
        assert_eq!(rec.committed_stage(), TransactionPoolStage::LocalPrepared);
        assert_eq!(rec.pending_stage(), None);

        tx.rollback().unwrap();
    }

    #[test]
    fn transaction_pool_get_all_overlays_pending_update_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        transaction_pool_get_all_overlays_pending_update(db);
    }

    // Regression: get_all must return each record's effective state, i.e. with the pending update from the
    // chain at the leaf block overlaid, not the bare committed record.
    fn transaction_pool_get_all_overlays_pending_update(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        let atom1 = create_tx_atom();

        let network = Network::LocalNet;
        let zero_block = Block::zero_block(network, num_preshards());
        zero_block.insert(&mut tx).unwrap();
        tx.proposal_certificates_save(zero_block.justify()).unwrap();
        tx.blocks_set_qcs(zero_block.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();

        let shard_group = zero_block.shard_group();

        let block1 = Block::create(
            network,
            ProtocolVersion::V0,
            *zero_block.id(),
            zero_block.justify().clone(),
            None,
            NodeHeight(1),
            Epoch(0),
            shard_group,
            Default::default(),
            [Command::LocalPrepare(atom1.clone())].into_iter().collect(),
            Default::default(),
            Default::default(),
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::default(),
        )
        .unwrap();
        block1.insert(&mut tx).unwrap();
        block1.as_locked().set(&mut tx).unwrap();
        block1.as_leaf().set(&mut tx).unwrap();

        tx.transaction_pool_insert_new(atom1.id, atom1.decision, &Evidence::empty(), true, false, Epoch(1), 0)
            .unwrap();

        // Base record has no pending update yet.
        let base = tx.transaction_pool_get_all(1000).unwrap();
        let base_rec = base.iter().find(|r| *r.id() == atom1.id).unwrap().clone();
        assert!(base_rec.pending_stage().is_none());

        // Record a pending transition for the transaction in block1, which is on the chain at the leaf.
        let mut updated = base_rec;
        updated
            .set_next_stage_and_readiness(TransactionPoolStage::LocalPrepared, shard_group)
            .unwrap();
        tx.transaction_pool_add_pending_update(&block1.as_leaf(), &TransactionPoolStatusUpdate::new(updated, true))
            .unwrap();

        let overlaid = tx.transaction_pool_get_all(1000).unwrap();
        let rec = overlaid.iter().find(|r| *r.id() == atom1.id).unwrap();
        assert_eq!(rec.pending_stage(), Some(TransactionPoolStage::LocalPrepared));

        tx.rollback().unwrap();
    }
}

mod transaction_operations {

    use super::*;

    #[test]
    fn transaction_operations_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        transaction_operations(db);
    }

    fn transaction_operations(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        // transactions_insert
        let tx1 = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(0)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&tx1).unwrap();
        let tx2 = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(1)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&tx2).unwrap();
        let unexisting_tx = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(2)))
                .build_and_seal(&PrivateKey::default()),
        );

        // transactions_get
        let res = tx.transactions_get(tx1.id()).unwrap();
        assert_eq_debug(&res, &tx1);
        let res = tx.transactions_get(tx2.id()).unwrap();
        assert_eq_debug(&res, &tx2);
        assert!(tx.transactions_get(unexisting_tx.id()).is_err());

        // transactions_exists
        let res = tx.transactions_exists(tx1.id()).unwrap();
        assert!(res);
        let res = tx.transactions_exists(tx2.id()).unwrap();
        assert!(res);
        let res = tx.transactions_exists(unexisting_tx.id()).unwrap();
        assert!(!res);

        // transactions_update
        let updated_tx = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(3)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&updated_tx).unwrap();

        let res = tx.transactions_get(updated_tx.id()).unwrap();
        assert_eq_debug(&res, &updated_tx);

        // transactions_get_any
        let res = tx
            .transactions_get_any(vec![tx1.id(), tx2.id(), unexisting_tx.id()])
            .unwrap();
        assert_eq!(res.len(), 2);

        // transactions_get_paginated
        // let res = tx.transactions_get_paginated(10, 0, None).unwrap();
        // assert_eq!(res.len(), 3);

        tx.rollback().unwrap();
    }
}

mod transaction_execution_operations {
    use tari_engine_types::fees::FeeReceiptBuilder;

    use super::*;

    #[test]
    fn transaction_execution_operations_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        transaction_execution_operations(db);
    }

    #[expect(clippy::too_many_lines)]
    fn transaction_execution_operations(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        // insert some transactions
        let tx1 = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(0)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&tx1).unwrap();
        let tx2 = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(1)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&tx2).unwrap();

        // insert blocks
        let chain = create_chain(10);
        commit_chain(&mut tx, &chain);

        let not_committed_block = chain[9].clone();
        // insert transaction executions
        let exec1 = BlockTransactionExecution::new(
            not_committed_block.as_leaf(),
            *tx1.id(),
            ExecuteResult {
                finalize: FinalizeResult::new(
                    Hash32::default(),
                    vec![],
                    vec![],
                    TransactionResult::Accept(SubstateDiff::new()),
                    FeeReceiptBuilder {
                        total_fee_payment: 0,
                        total_fees_paid: 0,
                        total_fee_overcharge: 0,
                        cost_breakdown: FeeBreakdown::default(),
                        exhaust_burn: 0,
                    }
                    .build(),
                ),
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            },
            vec![],
            vec![],
        );
        tx.block_transaction_executions_insert_or_ignore(&exec1).unwrap();

        // A committed ancestor of the queried block - its execution is still reusable (e.g. multishard accept).
        let committed_block = chain[6].clone();
        // insert transaction executions
        let exec2 = BlockTransactionExecution::new(
            committed_block.as_leaf(),
            *tx2.id(),
            ExecuteResult {
                finalize: FinalizeResult::new(
                    Hash32::default(),
                    vec![],
                    vec![],
                    TransactionResult::Accept(SubstateDiff::new()),
                    FeeReceiptBuilder {
                        total_fee_payment: 0,
                        total_fees_paid: 0,
                        total_fee_overcharge: 0,
                        cost_breakdown: FeeBreakdown::default(),
                        exhaust_burn: 0,
                    }
                    .build(),
                ),
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            },
            vec![],
            vec![],
        );
        assert!(tx.block_transaction_executions_insert_or_ignore(&exec2).unwrap());

        // transaction_executions_get_pending_for_block
        let res = tx
            .block_transaction_executions_get_pending_for_block(tx1.id(), &not_committed_block.as_leaf())
            .unwrap();
        assert_eq_debug(&res, &exec1);

        // block_transaction_executions_get_all_for_block
        let all = tx
            .block_transaction_executions_get_all_for_block(not_committed_block.id())
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq_debug(&all[0], &exec1);
        let all = tx
            .block_transaction_executions_get_all_for_block(committed_block.id())
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq_debug(&all[0], &exec2);
        let all = tx
            .block_transaction_executions_get_all_for_block(chain[3].id())
            .unwrap();
        assert!(all.is_empty());

        // transactions_finalize_all
        tx.transaction_pool_insert_new(
            *tx1.id(),
            Decision::Commit,
            &Evidence::empty(),
            true,
            false,
            Epoch(1),
            0,
        )
        .unwrap();
        let transactions = tx.transaction_pool_get_all(1000).unwrap();
        assert_eq!(transactions.len(), 1);
        tx.transactions_finalize_all(Epoch(1), transactions.iter()).unwrap();

        let rec = tx.transactions_get(tx1.id()).unwrap();
        assert!(rec.is_finalized(&*tx).unwrap(), "Transaction should be finalized");

        // Finalizing must not orphan the block index: block-scoped cascades and block introspection still need to
        // reach a finalized transaction's execution through the block it was executed in.
        let all = tx
            .block_transaction_executions_get_all_for_block(not_committed_block.id())
            .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq_debug(&all[0], &exec1);

        let pending = tx
            .block_transaction_executions_get_pending_for_block(tx2.id(), &not_committed_block.as_leaf())
            .unwrap();
        assert_eq!(*pending.transaction_id(), *tx2.id());

        // block_transaction_executions_lock_any_for_block
        tx.block_transaction_executions_lock_any_for_block(&not_committed_block.as_leaf())
            .unwrap();

        tx.rollback().unwrap();
    }

    #[test]
    fn it_excludes_orphan_block_executions_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        it_excludes_orphan_block_executions(db);
    }

    // Regression: an execution recorded on a block that is NOT part of the queried block's pending chain (e.g. an
    // abandoned/orphan branch left over across a restart) must never be returned, otherwise a stale execution pinned
    // to already-spent input versions can be reused.
    fn it_excludes_orphan_block_executions(db: impl StateStore) {
        use tari_ootle_storage::StorageError;

        use crate::helpers::create_block_with_qc;

        let mut tx = db.create_write_tx().unwrap();

        let tx1 = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(0)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&tx1).unwrap();

        let chain = create_chain(10);
        commit_chain(&mut tx, &chain);
        let leaf = chain.last().unwrap().as_leaf();

        // Fork off an in-chain block: this block is not on `leaf`'s parent chain, so it is an orphan w.r.t. `leaf`.
        let orphan_block = create_block_with_qc(&chain[5].as_leaf());
        orphan_block.insert(&mut tx).unwrap();

        // The transaction's only execution lives on the orphan block.
        let exec = BlockTransactionExecution::new(
            orphan_block.as_leaf(),
            *tx1.id(),
            ExecuteResult {
                finalize: FinalizeResult::new(
                    Hash32::default(),
                    vec![],
                    vec![],
                    TransactionResult::Accept(SubstateDiff::new()),
                    FeeReceiptBuilder {
                        total_fee_payment: 0,
                        total_fees_paid: 0,
                        total_fee_overcharge: 0,
                        cost_breakdown: FeeBreakdown::default(),
                        exhaust_burn: 0,
                    }
                    .build(),
                ),
                execution_time: Duration::from_secs(1),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            },
            vec![],
            vec![],
        );
        assert!(tx.block_transaction_executions_insert_or_ignore(&exec).unwrap());

        // It must not be returned for the canonical chain leaf - it belongs to an abandoned branch.
        let res = tx.block_transaction_executions_get_pending_for_block(tx1.id(), &leaf);
        assert!(
            matches!(res, Err(StorageError::NotFound { .. })),
            "orphan-branch execution must not be reused, got {res:?}"
        );

        tx.rollback().unwrap();
    }
}

mod finalized_transaction_gc {
    use super::*;

    fn insert_transaction(tx: &mut impl StateStoreWriteTransaction) -> TransactionRecord {
        let rec = TransactionRecord::new(
            Transaction::builder_localnet(Epoch(1))
                .add_instruction(Instruction::DropAllProofsInWorkspace)
                .add_input(SubstateRequirement::new(create_random_substate_id(), Some(0)))
                .build_and_seal(&PrivateKey::default()),
        );
        tx.transactions_insert(&rec).unwrap();
        rec
    }

    #[test]
    fn epoch_gc_prunes_finalized_transactions_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        epoch_gc_prunes_finalized_transactions(db);
    }

    fn epoch_gc_prunes_finalized_transactions(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        let chain = create_chain(10);
        commit_chain(&mut tx, &chain);

        let tx1 = insert_transaction(&mut tx);
        tx.transaction_pool_insert_new(
            *tx1.id(),
            Decision::Commit,
            &Evidence::empty(),
            true,
            false,
            Epoch(1),
            0,
        )
        .unwrap();
        let pool = tx.transaction_pool_get_all(1000).unwrap();
        tx.transactions_finalize_all(Epoch(1), pool.iter()).unwrap();
        assert!(tx.transactions_exists(tx1.id()).unwrap());
        assert!(TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());

        // epoch_history_length is 1 (default options): cleaning at epoch 1 prunes ..=0, which keeps
        // bookkeeping finalized in epoch 1.
        tx.epoch_cleanup(Epoch(1)).unwrap();
        assert!(tx.transactions_exists(tx1.id()).unwrap());
        assert!(TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());

        // Cleaning at epoch 2 prunes ..=1: payload, finalized link and executions all go.
        tx.epoch_cleanup(Epoch(2)).unwrap();
        assert!(!tx.transactions_exists(tx1.id()).unwrap());
        assert!(!TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());
        assert!(tx.finalized_transaction_execution_get(tx1.id()).is_err());

        tx.rollback().unwrap();
    }

    #[test]
    fn archival_node_keeps_transaction_history_rocksdb() {
        use helpers::create_rocksdb_with_opts;
        use tari_state_store_rocksdb::DatabaseOptions;

        let (db, _tmp) = create_rocksdb_with_opts(DatabaseOptions::default().with_prune_transaction_history(false));
        let mut tx = db.create_write_tx().unwrap();

        let chain = create_chain(10);
        commit_chain(&mut tx, &chain);

        let tx1 = insert_transaction(&mut tx);
        tx.transaction_pool_insert_new(
            *tx1.id(),
            Decision::Commit,
            &Evidence::empty(),
            true,
            false,
            Epoch(1),
            0,
        )
        .unwrap();
        let pool = tx.transaction_pool_get_all(1000).unwrap();
        tx.transactions_finalize_all(Epoch(1), pool.iter()).unwrap();

        tx.epoch_cleanup(Epoch(10)).unwrap();
        assert!(tx.transactions_exists(tx1.id()).unwrap());
        assert!(TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());

        tx.rollback().unwrap();
    }

    #[test]
    fn refinalized_transaction_ages_from_its_latest_epoch_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        refinalized_transaction_ages_from_its_latest_epoch(db);
    }

    /// The epoch index must hold exactly one entry per id, keyed to the latest finalization: a
    /// stale entry from an earlier finalization (or one left behind by
    /// `transactions_finalized_remove`) would let epoch GC delete the bookkeeping of a transaction
    /// that finalized recently. Survival across the earlier epochs' cleanup is the proof that the
    /// stale entries are gone.
    fn refinalized_transaction_ages_from_its_latest_epoch(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        let chain = create_chain(10);
        commit_chain(&mut tx, &chain);

        let tx1 = insert_transaction(&mut tx);
        tx.transaction_pool_insert_new(
            *tx1.id(),
            Decision::Commit,
            &Evidence::empty(),
            true,
            false,
            Epoch(1),
            0,
        )
        .unwrap();
        let pool = tx.transaction_pool_get_all(1000).unwrap();

        // Finalized in epoch 1, then finalized again in epoch 5: the index entry moves.
        tx.transactions_finalize_all(Epoch(1), pool.iter()).unwrap();
        tx.transactions_finalize_all(Epoch(5), pool.iter()).unwrap();
        tx.epoch_cleanup(Epoch(2)).unwrap();
        assert!(
            tx.transactions_exists(tx1.id()).unwrap(),
            "an id re-finalized in a later epoch must not age out from its earlier epoch"
        );
        assert!(TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());

        // Resurrected (as consensus does for an aborted id) and finalized again in epoch 7.
        tx.transactions_finalized_remove(tx1.id()).unwrap();
        assert!(!TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());
        tx.transactions_finalize_all(Epoch(7), pool.iter()).unwrap();
        tx.epoch_cleanup(Epoch(6)).unwrap();
        assert!(
            tx.transactions_exists(tx1.id()).unwrap(),
            "a resurrected id must not age out from the epoch of the removed finalization"
        );

        // It ages out from its latest finalization epoch.
        tx.epoch_cleanup(Epoch(8)).unwrap();
        assert!(!tx.transactions_exists(tx1.id()).unwrap());
        assert!(!TransactionRecord::is_record_finalized(&*tx, tx1.id()).unwrap());

        tx.rollback().unwrap();
    }
}

mod get_many_ready_weight_budget {
    use tari_consensus_types::BlockId;
    use tari_ootle_transaction::Network;
    use tari_template_lib_types::crypto::SchnorrSignatureBytes;

    use super::*;
    use crate::helpers::num_preshards;

    /// Insert `weights.len()` ready (New stage) transactions with the given static weights and return
    /// the block id to query against.
    fn setup_ready_pool(db: &impl StateStore, weights: &[u64]) -> BlockId {
        let mut tx = db.create_write_tx().unwrap();
        let network = Network::LocalNet;
        let zero_block = Block::zero_block(network, num_preshards());
        zero_block.insert(&mut tx).unwrap();
        tx.proposal_certificates_save(zero_block.justify()).unwrap();
        tx.blocks_set_qcs(zero_block.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();
        let shard_group = zero_block.shard_group();

        let atoms: Vec<_> = weights.iter().map(|_| create_tx_atom()).collect();
        let block1 = Block::create(
            network,
            ProtocolVersion::V0,
            *zero_block.id(),
            zero_block.justify().clone(),
            None,
            NodeHeight(1),
            Epoch(0),
            shard_group,
            Default::default(),
            // Need at least one command so the block causes a state change and is queryable.
            [Command::LocalPrepare(atoms[0].clone())].into_iter().collect(),
            Default::default(),
            Default::default(),
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::default(),
        )
        .unwrap();
        block1.insert(&mut tx).unwrap();
        block1.as_locked().set(&mut tx).unwrap();
        block1.as_leaf().set(&mut tx).unwrap();

        for (atom, weight) in atoms.iter().zip(weights) {
            tx.transaction_pool_insert_new(
                atom.id,
                atom.decision,
                &Evidence::empty(),
                true,
                false,
                Epoch(1),
                *weight,
            )
            .unwrap();
        }
        let block_id = *block1.id();
        tx.commit().unwrap();
        block_id
    }

    #[test]
    fn it_stops_when_weight_budget_exhausted_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        // Three New-stage transactions, each weight 100 (proposal_weight == 100 at New stage).
        let block_id = setup_ready_pool(&db, &[100, 100, 100]);
        let tx = db.create_read_tx().unwrap();

        // Budget for 2 (100 + 100 = 200 <= 250; the third would push to 300 > 250).
        let recs = tx.transaction_pool_get_many_ready(250, 10, &block_id).unwrap();
        assert_eq!(recs.len(), 2);

        // Generous budget fits all three.
        let recs = tx.transaction_pool_get_many_ready(u64::MAX, 10, &block_id).unwrap();
        assert_eq!(recs.len(), 3);
    }

    #[test]
    fn it_always_returns_at_least_one_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        // A single transaction heavier than the whole budget must still be returned so consensus
        // makes progress.
        let block_id = setup_ready_pool(&db, &[1000, 1000]);
        let tx = db.create_read_tx().unwrap();

        let recs = tx.transaction_pool_get_many_ready(10, 10, &block_id).unwrap();
        assert_eq!(recs.len(), 1);
    }

    #[test]
    fn it_respects_the_hard_count_cap_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        let block_id = setup_ready_pool(&db, &[1, 1, 1, 1]);
        let tx = db.create_read_tx().unwrap();

        // Weight is effectively unbounded but the count cap limits the batch.
        let recs = tx.transaction_pool_get_many_ready(u64::MAX, 2, &block_id).unwrap();
        assert_eq!(recs.len(), 2);
    }
}
