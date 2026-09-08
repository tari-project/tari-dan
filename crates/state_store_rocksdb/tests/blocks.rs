//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{commit_chain, create_chain, create_rocksdb, create_tx_atom};
use tari_common_types::types::FixedHash;
use tari_consensus_types::{PcId, ShardGroupAccumulatedData};
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    ExtraFieldKey,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    optional::Optional,
};
use tari_ootle_storage::{
    Ordering,
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    consensus_models::{Block, BookkeepingModel, Command},
};
use tari_ootle_transaction::Network;
use tari_template_lib_types::crypto::SchnorrSignatureBytes;
use tari_utilities::epoch_time::EpochTime;

mod basic_block_operations {
    use super::*;

    #[test]
    fn basic_block_operations_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        basic_block_operations(db);
    }

    fn basic_block_operations(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        // insert multiple blocks
        let mut chain = create_chain(3);
        commit_chain(&mut tx, &chain);
        let block1 = chain.pop().unwrap();
        let block2 = chain.pop().unwrap();
        let zero_block = chain.pop().unwrap();

        // fetch blocks by id
        let res = tx.blocks_get(zero_block.id()).unwrap();
        assert_eq!(res.id(), zero_block.id());
        let res = tx.blocks_get(block1.id()).unwrap();
        assert_eq!(res.calculate_id(), *block1.id());

        // blocks exist method
        assert!(tx.blocks_exists(zero_block.id()).unwrap());
        assert!(tx.blocks_exists(block1.id()).unwrap());

        // get all blocks
        // let res = tx.blocks_get_count().unwrap();
        // assert_eq!(res, 2);

        // set qcs
        let block1_from_db = tx.blocks_get(block1.id()).unwrap();
        assert!(!block1_from_db.has_justify_qc());
        tx.blocks_set_qcs(block1_from_db.id(), None, Some(&PcId::zero()))
            .unwrap();
        let block1_from_db = tx.blocks_get(block1.id()).unwrap();
        assert!(block1_from_db.has_justify_qc());

        // set is_commited flag
        let block1_from_db = tx.blocks_get(block1.id()).unwrap();
        assert!(!block1_from_db.is_committed());
        tx.blocks_set_qcs(block1_from_db.id(), Some(&PcId::zero()), None)
            .unwrap();
        let block1_from_db = tx.blocks_get(block1.id()).unwrap();
        assert!(block1_from_db.is_committed());

        let _ignore = tx.blocks_get(block2.id()).unwrap();
        tx.blocks_delete(block2.id()).unwrap();
        assert!(tx.blocks_get(block2.id()).optional().unwrap().is_none());
        assert!(!tx.blocks_exists(block2.id()).unwrap());

        tx.rollback().unwrap();
    }
}

mod block_parent_operations {
    use super::*;

    #[test]
    fn block_parent_operations_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        block_parent_operations(db);
    }

    fn block_parent_operations(db: impl StateStore) {
        let mut tx = db.create_write_tx().unwrap();

        // insert multiple blocks
        let network = Network::LocalNet;
        let atom1 = create_tx_atom();
        let atom2 = create_tx_atom();

        let zero_block = Block::zero_block(network, NumPreshards::P64);
        zero_block.insert(&mut tx).unwrap();

        let shard_group = ShardGroup::all_shards(NumPreshards::P64);
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

        let block2 = Block::create(
            network,
            ProtocolVersion::V0,
            *block1.id(),
            block1.justify().clone(),
            None,
            NodeHeight(1),
            Epoch(0),
            shard_group,
            Default::default(),
            // Need to have a command in, otherwise this block will not be included internally in the query because it
            // cannot cause a state change without any commands
            [Command::LocalPrepare(atom2.clone())].into_iter().collect(),
            Default::default(),
            Default::default(),
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::default(),
        )
        .unwrap();
        block2.insert(&mut tx).unwrap();

        // Some functions expect a locked block to be present
        block2.as_locked().set(&mut tx).unwrap();
        // TODO: this method needs to either be cleaned up or removed
        let blocks = tx.blocks_get_paginated(1000, 0, None, None, None, None).unwrap();
        assert_eq!(blocks.len(), 3);

        // check that all blocks are inserted
        let res = tx.blocks_get(zero_block.id()).unwrap();
        assert_eq!(res.id(), zero_block.id());
        let res = tx.blocks_get(block1.id()).unwrap();
        assert_eq!(res.calculate_id(), *block1.id());
        let res = tx.blocks_get(block2.id()).unwrap();
        assert_eq!(res.calculate_id(), *block2.id());

        // blocks_is_ancestor
        let res = tx.blocks_is_pending_ancestor(block2.id(), block1.id()).unwrap();
        assert!(res);
        let res = tx.blocks_is_pending_ancestor(block2.id(), zero_block.id()).unwrap();
        assert!(res);
        let res = tx.blocks_is_pending_ancestor(block1.id(), zero_block.id()).unwrap();
        assert!(res);
        let res = tx.blocks_is_pending_ancestor(block1.id(), block2.id()).unwrap();
        assert!(!res);
        let res = tx.blocks_is_pending_ancestor(block2.id(), block2.id()).unwrap();
        assert!(!res);

        // blocks_get_ids_by_parent
        // let res = tx.blocks_get_pending_ids_by_parent(zero_block.id()).unwrap();
        // assert_eq!(res, vec![*block1.id()]);
        let res = tx.blocks_get_pending_ids_by_parent(block1.id()).unwrap();
        assert_eq!(res, vec![*block2.id()]);
        let res = tx.blocks_get_pending_ids_by_parent(block2.id()).unwrap();
        assert_eq!(res, vec![]);

        // commit the blocks
        tx.blocks_set_qcs(zero_block.id(), Some(&PcId::zero()), None).unwrap();
        tx.blocks_set_qcs(block1.id(), Some(&PcId::zero()), None).unwrap();
        tx.blocks_set_qcs(block2.id(), Some(&PcId::zero()), None).unwrap();

        // blocks_get_all_by_parent
        let res = tx.blocks_get_committed_by_parent(zero_block.id()).unwrap();
        assert_eq!(res.calculate_id(), *block1.id());
        let res = tx.blocks_get_committed_by_parent(block1.id()).unwrap();
        assert_eq!(res.calculate_id(), *block2.id());
        let res = tx.blocks_get_committed_by_parent(block2.id()).optional().unwrap();
        assert!(res.is_none());

        // blocks_get_parent_chain
        // let res = tx.blocks_get_parent_chain(zero_block.id(), 10).unwrap();
        // assert_eq!(res.len(), 0);
        // let res = tx.blocks_get_parent_chain(block1.id(), 10).unwrap();
        // assert_eq!(res.len(), 0);
        // let res = tx.blocks_get_parent_chain(block2.id(), 10).unwrap();
        // assert_eq!(res.len(), 1);
        // assert_eq!(res[0].to_string(), block1.to_string());
        // TODO: test a longer parent chain

        // TODO: have a block with multiple children and check method results
        // TODO: remove block1 and check method results

        tx.rollback().unwrap();
    }
}

mod block_query_operations {
    use tari_ootle_transaction::Network;
    use tari_template_lib_types::crypto::SchnorrSignatureBytes;

    use super::*;

    #[test]
    #[allow(clippy::too_many_lines)]
    fn block_query_operations_rocksdb() {
        let (db, _tmp) = create_rocksdb();
        let mut tx = db.create_write_tx().unwrap();

        // insert multiple blocks
        let network = Network::LocalNet;
        let atom1 = create_tx_atom();
        let atom2 = create_tx_atom();

        let zero_block = Block::zero_block(network, NumPreshards::P64);
        zero_block.insert(&mut tx).unwrap();
        tx.blocks_set_qcs(zero_block.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();

        let shard_group = ShardGroup::all_shards(NumPreshards::P64);
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
        tx.blocks_set_qcs(block1.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();
        block1.as_locked().set(&mut tx).unwrap();

        let block2 = Block::create(
            network,
            ProtocolVersion::V0,
            *block1.id(),
            block1.justify().clone(),
            None,
            NodeHeight(2),
            Epoch(0),
            shard_group,
            Default::default(),
            // Need to have a command in, otherwise this block will not be included internally in the query because it
            // cannot cause a state change without any commands
            [Command::LocalPrepare(atom2.clone())].into_iter().collect(),
            Default::default(),
            // adding some fee to test blocks_get_total_leader_fee_for_epoch
            4,
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::default(),
        )
        .unwrap();
        block2.insert(&mut tx).unwrap();
        tx.blocks_set_qcs(block2.id(), Some(&PcId::zero()), Some(&PcId::zero()))
            .unwrap();
        tx.proposal_certificates_save(block2.justify()).unwrap();

        let mut block3_data = ExtraData::new();
        block3_data.insert(
            ExtraFieldKey::SidechainId,
            "block3".as_bytes().to_vec().try_into().unwrap(),
        );
        let block3 = Block::create(
            network,
            ProtocolVersion::V0,
            *block1.id(),
            block1.justify().clone(),
            None,
            // Height 2 to test forks
            NodeHeight(2),
            Epoch(0),
            shard_group,
            Default::default(),
            // Need to have a command in, otherwise this block will not be included internally in the query because it
            // cannot cause a state change without any commands
            [Command::LocalPrepare(atom2.clone())].into_iter().collect(),
            Default::default(),
            // adding some fee to test blocks_get_total_leader_fee_for_epoch
            5,
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            block3_data.clone(),
        )
        .unwrap();
        block3.insert(&mut tx).unwrap();
        block3.as_leaf().set(&mut tx).unwrap();

        // blocks_get_all_ids_by_height
        let res = tx.blocks_get_all_ids_by_height(Epoch(0), NodeHeight(1)).unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0], *block1.id());
        let res = tx.blocks_get_all_ids_by_height(Epoch(0), NodeHeight(2)).unwrap();
        assert_eq!(res.len(), 2);

        // blocks_get_genesis_for_epoch
        let res = tx.blocks_get_genesis_for_epoch(Epoch(0)).unwrap();
        assert_eq!(res.to_string(), zero_block.to_string());
        // TODO: try with another epoch

        let res = tx
            .blocks_get_all_between(Epoch(0), NodeHeight(0), NodeHeight(1), true, 10)
            .unwrap();
        assert_eq!(res.len(), 2);
        assert_eq!(res[0].id(), zero_block.id());
        assert_eq!(res[1].calculate_id(), *block1.id());

        // blocks_get_any_with_epoch_range
        // let res = tx
        //     .blocks_get_any_with_epoch_range(RangeInclusive::new(Epoch(0), Epoch(0)), None)
        //     .unwrap();
        // assert_eq!(res.len(), 4);

        // TODO: test with a greater epoch range
        // TODO: test with a specific vn key

        // blocks_get_paginated
        let res = tx.blocks_get_paginated(10, 0, None, None, None, None).unwrap();
        assert_eq!(res.len(), 4);
        // Filter by height
        let res = tx
            .blocks_get_paginated(10, 0, Some(2), Some("1".to_string()), None, None)
            .unwrap();
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].calculate_id(), *block1.id());
        let res = tx
            .blocks_get_paginated(1, 0, None, None, Some(7), Some(Ordering::Descending))
            .unwrap();
        assert_eq!(res.len(), 1);

        // filtered_blocks_get_count
        let res = tx.filtered_blocks_get_count(None, None).unwrap();
        assert_eq!(res, 4);
        let res = tx.filtered_blocks_get_count(Some(2), Some(1_u64.to_string())).unwrap();
        assert_eq!(res, 1);

        tx.rollback().unwrap();
    }
}
