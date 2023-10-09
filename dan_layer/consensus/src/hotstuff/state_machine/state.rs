//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::Display;

use crate::hotstuff::state_machine::{check_sync::CheckSync, idle::IdleState, running::Running, syncing::Syncing};

#[derive(Debug)]
pub(super) enum ConsensusState<TSpec> {
    Idle(IdleState<TSpec>),
    CheckSync(CheckSync<TSpec>),
    Syncing(Syncing<TSpec>),
    Running(Running<TSpec>),
    Sleeping,
    Shutdown,
}

#[derive(Debug, Clone, Copy, Default)]
pub enum ConsensusCurrentState {
    #[default]
    Idle,
    CheckSync,
    Syncing,
    Running,
    Sleeping,
    Shutdown,
}

impl ConsensusCurrentState {
    pub fn is_running(&self) -> bool {
        matches!(self, ConsensusCurrentState::Running)
    }
}

impl<TSpec> ConsensusState<TSpec> {
    pub fn is_shutdown(&self) -> bool {
        matches!(self, ConsensusState::Shutdown)
    }
}

impl<TSpec> Display for ConsensusState<TSpec> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        #[allow(clippy::enum_glob_use)]
        use ConsensusState::*;
        match self {
            Idle(_) => write!(f, "Idle"),
            CheckSync(_) => write!(f, "CheckSync"),
            Syncing(_) => write!(f, "Syncing"),
            Running(_) => write!(f, "Running"),
            Sleeping => write!(f, "Sleeping"),
            Shutdown => write!(f, "Shutdown"),
        }
    }
}

impl<TSpec> From<&ConsensusState<TSpec>> for ConsensusCurrentState {
    fn from(value: &ConsensusState<TSpec>) -> Self {
        match value {
            ConsensusState::Idle(_) => ConsensusCurrentState::Idle,
            ConsensusState::CheckSync(_) => ConsensusCurrentState::CheckSync,
            ConsensusState::Syncing(_) => ConsensusCurrentState::Syncing,
            ConsensusState::Running(_) => ConsensusCurrentState::Running,
            ConsensusState::Sleeping => ConsensusCurrentState::Sleeping,
            ConsensusState::Shutdown => ConsensusCurrentState::Shutdown,
        }
    }
}
