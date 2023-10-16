//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display};

use tari_dan_common_types::Epoch;

use crate::hotstuff::HotStuffError;

#[derive(Debug)]
pub enum ConsensusStateEvent {
    RegisteredForEpoch { epoch: Epoch },
    NotRegisteredForEpoch { epoch: Epoch },
    NeedSync,
    SyncComplete,
    Ready,
    Failure { error: HotStuffError },
    Resume,
    Shutdown,
}

impl Display for ConsensusStateEvent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        #[allow(clippy::enum_glob_use)]
        use ConsensusStateEvent::*;
        match self {
            RegisteredForEpoch { epoch } => write!(f, "Registered for epoch {}", epoch),
            NotRegisteredForEpoch { epoch } => write!(f, "Not registered for epoch {}", epoch),
            NeedSync => write!(f, "Behind peers"),
            SyncComplete => write!(f, "Sync complete"),
            Ready => write!(f, "Ready"),
            Failure { error } => write!(f, "Failure({error})"),
            Resume => write!(f, "Resume"),
            Shutdown => write!(f, "Shutdown"),
        }
    }
}
