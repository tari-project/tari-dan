//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::NodeHeight;
use tari_dan_storage::consensus_models::QuorumCertificate;

use super::VoteMessage;

#[derive(Debug, Clone, Serialize)]
pub struct NewViewMessage {
    pub high_qc: QuorumCertificate,
    pub new_height: NodeHeight,
    pub last_vote: Option<VoteMessage>,
}
