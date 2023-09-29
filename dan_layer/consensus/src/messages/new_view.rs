//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::{Epoch, NodeHeight};
use tari_dan_storage::consensus_models::QuorumCertificate;

#[derive(Debug, Clone, Serialize)]
pub struct NewViewMessage<TAddr> {
    pub high_qc: QuorumCertificate<TAddr>,
    pub epoch: Epoch,
    pub new_height: NodeHeight,
}
