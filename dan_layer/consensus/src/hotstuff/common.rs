//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::ops::DerefMut;

use log::*;
use tari_dan_common_types::{committee::Committee, NodeAddressable};
use tari_dan_storage::{
    consensus_models::{HighQc, QuorumCertificate},
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
};

use crate::messages::HotstuffMessage;

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff";

/// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
/// This is a placeholder for the fee exhaust consensus constant so that we know where it's used later.
/// TODO: exhaust > 0
pub const EXHAUST_DIVISOR: u64 = 0;

// To avoid clippy::type_complexity
pub(super) type CommitteeAndMessage<TAddr> = (Committee<TAddr>, HotstuffMessage<TAddr>);

pub fn update_high_qc<TTx, TAddr: NodeAddressable>(
    tx: &mut TTx,
    qc: &QuorumCertificate<TAddr>,
) -> Result<(), StorageError>
where
    TTx: StateStoreWriteTransaction<Addr = TAddr> + DerefMut,
    TTx::Target: StateStoreReadTransaction,
{
    let high_qc = HighQc::get(tx.deref_mut())?;
    let high_qc = high_qc.get_quorum_certificate(tx.deref_mut())?;

    if high_qc.block_height() < qc.block_height() {
        debug!(
            target: LOG_TARGET,
            "🔥 UPDATE_HIGH_QC (node: {} {}, previous high QC: {} {})",
            qc.id(),
            qc.block_height(),
            high_qc.block_id(),
            high_qc.block_height(),
        );

        qc.save(tx)?;
        // This will fail if the block doesnt exist
        qc.as_leaf_block().set(tx)?;
        qc.as_high_qc().set(tx)?;
    }

    Ok(())
}
