//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_networking::NetworkingError;

#[derive(Debug, thiserror::Error)]
pub enum MessagingError {
    #[error("Failed to send to loopback because channel was closed")]
    LoopbackSendFailed,
    #[error("Networking error: {0}")]
    NetworkingError(#[from] NetworkingError),
}
