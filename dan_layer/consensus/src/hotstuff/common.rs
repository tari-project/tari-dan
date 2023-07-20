//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_storage::{
    consensus_models::{HighQc, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};

use crate::hotstuff::error::HotStuffError;

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff";

pub fn update_high_qc<TTx>(tx: &mut TTx, qc: &QuorumCertificate) -> Result<(), HotStuffError>
where
    TTx: StateStoreWriteTransaction + DerefMut,
    TTx::Target: StateStoreReadTransaction,
{
    let high_qc = HighQc::get(tx.deref_mut(), qc.epoch())?;
    let high_qc = high_qc.get_quorum_certificate(tx.deref_mut())?;
    // high_qc.node
    let high_qc_block = high_qc.get_block(tx.deref_mut())?;

    if high_qc_block.height() < qc.block_height() {
        debug!(
            target: LOG_TARGET,
            "🔥 UPDATE_HIGH_QC (node: {} {}, previous high QC: {} {})",
            qc.id(),
            qc.block_height(),
            high_qc_block.id(),
            high_qc_block.height(),
        );

        qc.set_block_as_leaf(tx)?;
        qc.save(tx)?;
        qc.set_as_high_qc(tx)?;
    }

    Ok(())
}
