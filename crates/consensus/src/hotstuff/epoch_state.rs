//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use tari_engine_types::fees::ExhaustBurnRate;
use tari_epoch_manager::{EpochManagerError, EpochManagerReader};
use tari_ootle_common_types::{
    Epoch,
    committee::{Committee, CommitteeInfo},
};
use tari_sidechain::FixedHash;

pub struct EpochState<TAddr> {
    /// The current epoch
    pub epoch: Epoch,
    pub epoch_hash: FixedHash,
    /// The share of collected fees burned rather than paid to leaders, for the whole of `epoch`.
    ///
    /// Read off the genesis block that opened `epoch`, which is the value the committee ratified
    /// when it committed the previous epoch's end-of-epoch block. Every block of the epoch must name
    /// it, which is what `check_exhaust_burn_rate` holds a proposal to.
    pub exhaust_burn_rate: ExhaustBurnRate,
    // /// The shard group for the local validator in the current epoch
    // pub registered_shard_group: Option<ShardGroup>,
    pub local_committee_info: CommitteeInfo,
    pub local_committee: Arc<Committee<TAddr>>,
}

impl<TAddr> EpochState<TAddr> {
    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    pub fn epoch_hash(&self) -> &FixedHash {
        &self.epoch_hash
    }

    pub fn exhaust_burn_rate(&self) -> ExhaustBurnRate {
        self.exhaust_burn_rate
    }

    pub fn local_committee_info(&self) -> &CommitteeInfo {
        &self.local_committee_info
    }

    pub fn local_committee(&self) -> &Committee<TAddr> {
        &self.local_committee
    }

    /// `exhaust_burn_rate` is read from the store by the caller rather than from the epoch manager:
    /// the rate is a layer-2 fact the committee ratified, and the oracle knows nothing about it.
    pub async fn update_from_epoch_manager<TEpochManager: EpochManagerReader<Addr = TAddr>>(
        &mut self,
        epoch_manager: &TEpochManager,
        current_epoch: Epoch,
        exhaust_burn_rate: ExhaustBurnRate,
    ) -> Result<(), EpochManagerError> {
        self.local_committee_info = epoch_manager.get_local_committee_info(current_epoch).await?;
        self.local_committee = epoch_manager.get_local_committee(current_epoch).await?;
        self.epoch_hash = epoch_manager.get_epoch_hash(current_epoch).await?;
        self.exhaust_burn_rate = exhaust_burn_rate;
        self.epoch = current_epoch;

        Ok(())
    }
}
