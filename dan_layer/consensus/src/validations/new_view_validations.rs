//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::warn;
use tari_dan_common_types::committee::{Committee, CommitteeInfo, CommitteeShardInfo};
use tari_dan_storage::consensus_models::LockedBlock;
use tari_epoch_manager::EpochManagerReader;

use crate::{
    hotstuff::{HotStuffError, NewViewValidationError},
    messages::NewViewMessage,
    traits::{ConsensusSpec, LeaderStrategy},
};
pub async fn check_new_view_message<TConsensusSpec: ConsensusSpec>(
    message: &NewViewMessage,
    epoch_manager: &TConsensusSpec::EpochManager,
    locked: &LockedBlock,
    leader_strategy: &TConsensusSpec::LeaderStrategy,
    local_committee: &Committee<TConsensusSpec::Addr>,
    local_committee_shard: &CommitteeInfo,
) -> Result<(), HotStuffError> {
    let epoch = message.epoch;
    if !epoch_manager
        .is_this_validator_registered_for_epoch(message.epoch)
        .await?
    {
        return Err(HotStuffError::NotRegisteredForCurrentEpoch { epoch });
    }

    if message.new_height < locked.height() {
        return Err(NewViewValidationError::NewViewHeightLessThanLockedBlock {
            locked_height: locked.height(),
            new_view_height: message.new_height,
        }.into());
    }

    // TODO: QC Validation

    let leader = leader_strategy.get_leader_for_next_block(local_committee, message.new_height);
    let our_node = epoch_manager.get_our_validator_node(epoch).await?;

    if *leader != our_node.address {
        // warn!(target: LOG_TARGET, "❌ New View failed, leader is {} at height:{}", leader, new_height);
        return Err(HotStuffError::NotTheLeader {
            details: format!(
                "Received NEWVIEW height {} but this not is not the leader for that height",
                message.new_height
            ),
        });
    }

    // Are nodes requesting to create more than the minimum number of dummy blocks?
    if message
        .high_qc
        .block_height()
        .saturating_sub(message.new_height)
        .as_u64() >
        local_committee.len() as u64
    {
        return Err(NewViewValidationError::BadNewViewMessage {
            details: format!(
                "HighQC block height is higher than the number of committee members. This is not allowed."
            ),
            high_qc_height: message.high_qc.block_height(),
            received_new_height: message.new_height,
        }.into());
    }

    Ok(())
}
