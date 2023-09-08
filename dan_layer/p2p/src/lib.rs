//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

mod message;
mod outbound_service;
mod peer_service;

pub use message::*;
pub use outbound_service::*;
pub use peer_service::*;
use tari_comms::protocol::ProtocolId;

pub static TARI_DAN_MSG_PROTOCOL_ID: ProtocolId = ProtocolId::from_static(b"t/msg/1.0");
