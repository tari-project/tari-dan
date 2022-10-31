//  Copyright 2021. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

pub mod base_layer_epoch_manager;
pub mod epoch_manager_service;
pub mod handle;

mod initializer;
use std::ops::RangeInclusive;

pub use initializer::spawn;
use tari_dan_common_types::ShardId;
use tari_dan_core::{models::ValidatorNode, services::epoch_manager::EpochManagerError};

pub fn get_committee_shard_ids(vns: &[ValidatorNode]) -> Result<RangeInclusive<ShardId>, EpochManagerError> {
    // TODO: add this committee_size to ConsensusConstants
    let committee_size = 7;
    if vns.len() < committee_size {
        let min_shard_id = ShardId::zero();
        let max_shard_id = ShardId([u8::MAX; 32]);
        Ok(RangeInclusive::new(min_shard_id, max_shard_id))
    } else {
        let min_shard_id = vns.first().ok_or(EpochManagerError::ValidatorNodesNotFound)?.shard_key;
        let max_shard_id = vns.last().ok_or(EpochManagerError::ValidatorNodesNotFound)?.shard_key;
        Ok(RangeInclusive::new(min_shard_id, max_shard_id))
    }
}
