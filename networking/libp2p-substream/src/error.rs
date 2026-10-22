//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Debug, Display, Formatter};

#[derive(Debug, Clone)]
pub enum Error {
    ConnectionClosed,
    DialFailure { details: String },
    NoAddressesForPeer,
    DialUpgradeError,
    ProtocolNotSupported,
    ProtocolNegotiationTimeout,
    ProtocolNegotiationFailed { details: String },
    ChannelClosed,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionClosed => write!(f, "Connection closed"),
            Self::DialFailure { details } => write!(f, "Dial failure: {details}"),
            Self::NoAddressesForPeer => write!(f, "No addresses for peer"),
            Self::DialUpgradeError => write!(f, "Dial upgrade error"),
            Self::ProtocolNotSupported => write!(f, "Protocol not supported"),
            Self::ProtocolNegotiationTimeout => write!(f, "Protocol negotiation timeout"),
            Self::ProtocolNegotiationFailed { details } => {
                write!(f, "Protocol negotiation with the peer failed: {details}")
            },
            Self::ChannelClosed => write!(f, "Channel closed"),
        }
    }
}

impl std::error::Error for Error {}
