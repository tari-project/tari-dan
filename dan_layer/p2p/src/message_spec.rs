//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_networking::MessageSpec;

use crate::proto;

#[derive(Debug, Clone, Copy)]
pub struct TariMessagingSpec;

impl MessageSpec for TariMessagingSpec {
    type TransactionGossipMessage = proto::network::DanMessage;
    type ConsensusGossipMessage = proto::consensus::HotStuffMessage;
    type Message = proto::consensus::HotStuffMessage;
}
