//  Copyright 2022. The Tari Project
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

use std::time::Duration;

use tari_common::configuration::Network;
use tari_dan_common_types::NumPreshards;

#[derive(Clone, Debug)]
pub struct ConsensusConstants {
    pub base_layer_confirmations: u64,
    pub committee_size: u32,
    pub max_base_layer_blocks_ahead: u64,
    pub max_base_layer_blocks_behind: u64,
    pub num_preshards: NumPreshards,
    pub pacemaker_block_time: Duration,
    /// The number of missed proposals before a SuspendNode proposal is sent
    pub missed_proposal_suspend_threshold: u64,
    /// The maximum number of missed proposals to count. If a peer is offline, gets suspended and comes online, their
    /// missed proposal count is decremented for each block that they participate in. Once this reaches zero, the node
    /// is considered online and will be reinstated. This cap essentially gives the maximum number of rounds until they
    /// will be reinstated once they resume participation in consensus.
    pub missed_proposal_count_cap: u64,
    pub max_block_size: usize,
    /// The value that fees are divided by to determine the amount of fees to burn. 0 means no fees are burned.
    pub fee_exhaust_divisor: u64,
    /// Maximum number of validator nodes to be activated in an epoch.
    /// This is to give enough time to the network to catch up with new validator nodes and do syncing.
    pub max_vns_per_epoch_activated: u64,
}

impl ConsensusConstants {
    pub const fn devnet() -> Self {
        Self {
            base_layer_confirmations: 3,
            committee_size: 7,
            max_base_layer_blocks_ahead: 5,
            max_base_layer_blocks_behind: 5,
            num_preshards: NumPreshards::P256,
            pacemaker_block_time: Duration::from_secs(10),
            missed_proposal_suspend_threshold: 5,
            missed_proposal_count_cap: 5,
            max_block_size: 500,
            fee_exhaust_divisor: 20, // 5%
            max_vns_per_epoch_activated: 50,
        }
    }
}

impl From<Network> for ConsensusConstants {
    fn from(network: Network) -> Self {
        match network {
            Network::MainNet => unimplemented!("Mainnet consensus constants not implemented"),
            Network::StageNet | Network::NextNet | Network::LocalNet | Network::Igor | Network::Esmeralda => {
                Self::devnet()
            },
        }
    }
}
