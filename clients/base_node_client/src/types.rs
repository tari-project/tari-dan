//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_common_types::types::{FixedHash, PublicKey};
use tari_core::transactions::{tari_amount::MicroMinotari, transaction_components::TransactionOutput};
use tari_dan_common_types::{Epoch, SubstateAddress};
#[cfg(feature = "ts")]
use ts_rs::TS;

#[derive(Debug, Clone)]
pub struct BaseLayerMetadata {
    pub height_of_longest_chain: u64,
    pub tip_hash: FixedHash,
}

#[derive(Debug, Clone)]
pub struct SideChainUtxos {
    pub block_info: BlockInfo,
    pub outputs: Vec<TransactionOutput>,
}

#[derive(Debug, Clone)]
pub struct BlockInfo {
    pub hash: FixedHash,
    pub height: u64,
    pub next_block_hash: Option<FixedHash>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct ValidatorNode {
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    pub public_key: PublicKey,
    pub shard_key: SubstateAddress,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BaseLayerConsensusConstants {
    pub validator_node_registration_expiry: u64,
    pub epoch_length: u64,
    pub validator_node_registration_min_deposit_amount: MicroMinotari,
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

    pub fn validator_node_registration_min_deposit_amount(&self) -> MicroMinotari {
        self.validator_node_registration_min_deposit_amount
    }

    pub fn epoch_length(&self) -> u64 {
        self.epoch_length
    }
}
