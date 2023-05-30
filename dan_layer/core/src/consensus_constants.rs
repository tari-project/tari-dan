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

use serde::{Deserialize, Serialize};
use tari_core::transactions::tari_amount::MicroTari;
use tari_dan_common_types::{Epoch, NodeHeight};

#[derive(Clone, Debug)]
pub struct ConsensusConstants {
    pub base_layer_confirmations: u64,
    pub committee_size: u64,
    pub hotstuff_rounds: u64,
}

impl ConsensusConstants {
    pub const fn devnet() -> Self {
        Self {
            base_layer_confirmations: 3,
            committee_size: 7,
            hotstuff_rounds: 4,
        }
    }

    pub fn max_payload_height(&self) -> NodeHeight {
        NodeHeight(self.hotstuff_rounds)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseLayerConsensusConstants {
    pub validator_node_registration_expiry: u64,
    pub epoch_length: u64,
    pub validator_node_registration_min_deposit_amount: MicroTari,
}

impl BaseLayerConsensusConstants {
    pub fn height_to_epoch(&self, height: u64) -> Epoch {
        Epoch(height / self.epoch_length)
    }

    pub fn epoch_to_height(&self, epoch: Epoch) -> u64 {
        epoch.0 * self.epoch_length
    }

    pub fn validator_node_registration_expiry(&self) -> Epoch {
        Epoch(self.validator_node_registration_expiry)
    }

    pub fn validator_node_registration_min_deposit_amount(&self) -> MicroTari {
        self.validator_node_registration_min_deposit_amount
    }

    pub fn epoch_length(&self) -> u64 {
        self.epoch_length
    }
}
