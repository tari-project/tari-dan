//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

mod bootstrap;
pub use bootstrap::*;
pub mod memory;

use std::{error::Error, fmt::Debug};

use tari_dan_common_types::optional::IsNotFoundError;
use tari_engine_types::substate::{Substate, SubstateId};

pub trait StateReader {
    fn get_state(&self, key: &SubstateId) -> Result<&Substate, StateStoreError>;
    fn exists(&self, key: &SubstateId) -> Result<bool, StateStoreError>;
}

pub trait StateWriter: StateReader {
    fn set_state(&mut self, key: SubstateId, value: Substate) -> Result<(), StateStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("Non existent shard: {shard:?}")]
    NonExistentShard { shard: Vec<u8> },
    #[error(transparent)]
    Custom(#[from] anyhow::Error),
    #[error("Error: {0}")]
    CustomStr(String),
    #[error("{kind} not found with key {key}")]
    NotFound { kind: &'static str, key: String },
    #[error("Substate has already been destroyed")]
    SubstateDestroyed,
}

impl StateStoreError {
    pub fn custom<E: Error + Sync + Send + 'static>(e: E) -> Self {
        StateStoreError::Custom(e.into())
    }

    pub fn custom_str(e: &str) -> Self {
        StateStoreError::CustomStr(e.to_string())
    }
}

impl IsNotFoundError for StateStoreError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, StateStoreError::NotFound { .. })
    }
}
