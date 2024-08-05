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
    pub pacemaker_max_base_time: std::time::Duration,
}

impl ConsensusConstants {
    pub const fn devnet() -> Self {
        Self {
            base_layer_confirmations: 3,
            committee_size: 7,
            max_base_layer_blocks_ahead: 5,
            max_base_layer_blocks_behind: 5,
            num_preshards: NumPreshards::SixtyFour,
            pacemaker_max_base_time: Duration::from_secs(10),
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
