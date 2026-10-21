//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub mod helpers;

use helpers::{assert_eq_debug, create_random_block_id, create_rocksdb};
use tari_consensus_types::ProposalCertificate;
use tari_ootle_common_types::{Epoch, NodeHeight, NumPreshards, ShardGroup};
use tari_ootle_storage::{StateStore, StateStoreReadTransaction, StateStoreWriteTransaction};
use tari_sidechain::QuorumDecision;

#[test]
fn quorum_certificates_rocksdb() {
    let (db, _tmp) = create_rocksdb();
    quorum_certificates_operations(db);
}

fn quorum_certificates_operations(db: impl StateStore) {
    let mut tx = db.create_write_tx().unwrap();

    let epoch = Epoch::zero();
    let shard_group = ShardGroup::all_shards(NumPreshards::P4);
    let genesis_qc = ProposalCertificate::genesis(epoch, shard_group);

    let block_id_1 = create_random_block_id();
    let qc1 = ProposalCertificate::new(
        *genesis_qc.header_hash(),
        block_id_1,
        genesis_qc.height() + NodeHeight(1),
        epoch,
        shard_group,
        ProposalCertificate::genesis_signatures(),
        QuorumDecision::Accept,
    );

    // insert both QCs in database
    tx.proposal_certificates_save(&genesis_qc).unwrap();
    tx.proposal_certificates_save(&qc1).unwrap();

    // quorum_certificates_get
    let res = tx.proposal_certificates_get(epoch, &genesis_qc.calculate_id()).unwrap();
    assert_eq_debug(&res, &genesis_qc);
    let res = tx.proposal_certificates_get(epoch, &qc1.calculate_id()).unwrap();
    assert_eq_debug(&res, &qc1);

    // quorum_certificates_get_all
    let qc_ids = vec![(epoch, genesis_qc.calculate_id()), (epoch, qc1.calculate_id())];
    let res = tx.proposal_certificates_get_many(&qc_ids).unwrap();
    assert_eq!(res.len(), 2);

    tx.rollback().unwrap();
}
