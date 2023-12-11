//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use tari_dan_common_types::Epoch;

#[derive(Debug, Clone, Serialize)]
pub struct RequestMissingForeignBlocksMessage {
    pub epoch: Epoch,
    pub from: u64,
    pub to: u64,
}
