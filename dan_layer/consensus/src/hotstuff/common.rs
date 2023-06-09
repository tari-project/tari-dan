//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_storage::{
    consensus_models::{HighQc, LeafBlock, QuorumCertificate},
    StateStore,
};

use crate::hotstuff::error::HotStuffError;

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff";

pub fn update_high_qc<TStore: StateStore>(
    tx: &mut TStore::WriteTransaction<'_>,
    qc: &QuorumCertificate,
) -> Result<(), HotStuffError> {
    let high_qc = HighQc::get(tx.deref_mut(), qc.epoch())?;
    let high_qc = high_qc.get_quorum_certificate(tx.deref_mut())?;
    // high_qc.node
    let high_qc_block = high_qc.get_block(tx.deref_mut())?;

    let new_qc_block = qc.get_block(tx.deref_mut())?;

    if high_qc_block.height() < new_qc_block.height() {
        debug!(
            target: LOG_TARGET,
            "🔥 UPDATE_HIGH_QC (node: {} {}, tx_count: {}, previous block: {} {})",
            new_qc_block.id(),
            new_qc_block.height(),
            new_qc_block.transaction_count(),
            high_qc_block.id(),
            high_qc_block.height(),
        );

        LeafBlock {
            epoch: new_qc_block.epoch(),
            block_id: *new_qc_block.id(),
            height: new_qc_block.height(),
        }
        .save(tx)?;

        HighQc {
            epoch: new_qc_block.epoch(),
            block_id: *new_qc_block.id(),
            height: new_qc_block.height(),
        }
        .save(tx)?;
    }

    Ok(())
}
