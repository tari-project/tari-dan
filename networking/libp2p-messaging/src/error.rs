//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Display, Formatter},
    io,
    time::Duration,
};

use futures_bounded::Timeout;

#[derive(Debug)]
pub enum Error {
    CodecError(io::Error),
    ConnectionClosed,
    Timeout(Timeout),
    ReceiveTimeout(Duration),
    SendTimeout(Duration),
    DialFailure,
    DialUpgradeError,
    ProtocolNegotiationFailed(io::Error),
    ProtocolNotSupported,
    ChannelClosed,
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::CodecError(err) => write!(f, "Codec error: {}", err),
            Self::ConnectionClosed => write!(f, "Connection closed"),
            Self::Timeout(err) => write!(f, "Timeout: {}", err),
            Self::ReceiveTimeout(timeout) => write!(f, "Peer did not finish sending a message within {timeout:?}"),
            Self::SendTimeout(timeout) => write!(f, "Peer did not accept a message within {timeout:?}"),
            Self::DialFailure => write!(f, "Dial failure"),
            Self::DialUpgradeError => write!(f, "Dial upgrade error"),
            Self::ProtocolNegotiationFailed(err) => write!(f, "Protocol negotiation with the peer failed: {err}"),
            Self::ProtocolNotSupported => write!(f, "Protocol not supported"),
            Self::ChannelClosed => write!(f, "Channel closed"),
        }
    }
}

impl std::error::Error for Error {}
