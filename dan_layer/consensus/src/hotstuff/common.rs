//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::committee::Committee;
use tari_dan_storage::{
    consensus_models::{HighQc, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
};

use crate::{hotstuff::error::HotStuffError, messages::HotstuffMessage};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff";

/// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
/// This is a placeholder for the fee exhaust consensus constant so that we know where it's used later.
/// TODO: exhaust > 0
pub const EXHAUST_DIVISOR: u64 = 0;

// To avoid clippy::type_complexity
pub(super) type CommitteeAndMessage<TAddr> = (Committee<TAddr>, HotstuffMessage<TAddr>);

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

        qc.as_leaf_block().set(tx)?;
        qc.save(tx)?;
        qc.as_high_qc().set(tx)?;
    }

    Ok(())
}
