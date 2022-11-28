//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#[derive(Debug, thiserror::Error)]
pub enum MessagingError {
    #[error("Failed to send to loopback because channel was closed")]
    LoopbackSendFailed,
    #[error("Failed to send to outbound messaging because the channel was closed")]
    MessageSendFailed,
}
