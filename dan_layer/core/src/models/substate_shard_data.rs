//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_dan_common_types::{
    quorum_certificate::QuorumCertificate,
    NodeHeight,
    PayloadId,
    ShardId,
    SubstateState,
    TreeNodeHash,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SubstateShardData {
    shard: ShardId,
    substate: SubstateState,
    height: NodeHeight,
    tree_node_hash: Option<TreeNodeHash>,
    payload_id: PayloadId,
    certificate: Option<QuorumCertificate>,
}

impl SubstateShardData {
    pub fn new(
        shard: ShardId,
        substate: SubstateState,
        height: NodeHeight,
        tree_node_hash: Option<TreeNodeHash>,
        payload_id: PayloadId,
        certificate: Option<QuorumCertificate>,
    ) -> Self {
        Self {
            shard,
            substate,
            height,
            tree_node_hash,
            payload_id,
            certificate,
        }
    }

    pub fn shard(&self) -> ShardId {
        self.shard
    }

    pub fn substate(&self) -> &SubstateState {
        &self.substate
    }

    pub fn height(&self) -> NodeHeight {
        self.height
    }

    pub fn tree_node_hash(&self) -> Option<TreeNodeHash> {
        self.tree_node_hash
    }

    pub fn payload_id(&self) -> PayloadId {
        self.payload_id
    }

    pub fn certificate(&self) -> &Option<QuorumCertificate> {
        &self.certificate
    }
}
