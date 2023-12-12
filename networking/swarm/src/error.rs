//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use libp2p::{noise, swarm::InvalidProtocol};

#[derive(Debug, thiserror::Error)]
pub enum TariSwarmError {
    #[error("Noise error: {0}")]
    Noise(#[from] noise::Error),
    #[error(transparent)]
    InvalidProtocol(#[from] InvalidProtocol),
    #[error("Behaviour error: {0}")]
    BehaviourError(String),
    #[error("Failed to parse protocol version field '{field}'")]
    ProtocolVersionParseFailed { field: &'static str },
}
