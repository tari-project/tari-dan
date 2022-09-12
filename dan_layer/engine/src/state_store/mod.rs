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

pub mod memory;

use std::{error::Error, io};

use tari_dan_common_types::SubstateState;
use tari_template_abi::{encode, Decode, Encode};

// pub trait StateStorage<'a>: AtomicDb<'a, Error = StateStoreError> + Send + Sync {}
//
// impl<'a, T> StateStorage<'a> for T
// where
//     T: AtomicDb<'a, Error = StateStoreError> + Send + Sync,
//     T::ReadAccess: StateReader,
//     T::WriteAccess: StateWriter,
// {
// }

/// Abstraction for any database that has atomic read/write semantics.
pub trait AtomicDb<'a> {
    type Error;
    type ReadAccess: 'a;
    type WriteAccess: 'a;

    /// Obtain read access to the underlying database
    fn read_access(&'a self) -> Result<Self::ReadAccess, Self::Error>;

    /// Obtain write access to the underlying database
    fn write_access(&'a self) -> Result<Self::WriteAccess, Self::Error>;
}

pub trait StateReader {
    fn get_state_raw(&self, key: &[u8]) -> Result<Option<Vec<u8>>, StateStoreError>;

    fn get_state_as_state<K: Encode>(&self, key: &K) -> Result<Option<SubstateState>, StateStoreError> {
        let value = self.get_state_raw(&encode(key)?)?;
        let value = value
            .map(|v| SubstateState::deserialize(&mut v.as_slice()))
            .transpose()?;
        Ok(value)
    }

    fn get_state<K: Encode, V: Decode>(&self, key: &K) -> Result<Option<V>, StateStoreError> {
        let state = self.get_state_as_state(key)?;
        if let Some(s) = state {
            match s {
                SubstateState::DoesNotExist => Ok(None),
                SubstateState::Exists { data, .. } => {
                    let value = V::deserialize(&mut data.as_slice())?;
                    Ok(Some(value))
                },
                SubstateState::Destroyed { .. } => Err(StateStoreError::SubstateDestroyed),
            }
        } else {
            Ok(None)
        }
    }

    fn exists(&self, key: &[u8]) -> Result<bool, StateStoreError>;
}

pub trait StateWriter: StateReader {
    fn set_state_raw(&mut self, key: &[u8], value: Vec<u8>) -> Result<(), StateStoreError>;

    fn set_state<K: Encode, V: Encode>(&mut self, key: &K, value: V) -> Result<(), StateStoreError> {
        self.set_state_raw(&encode(key)?, encode(&value)?)
    }

    fn commit(self) -> Result<(), StateStoreError>;
}

#[derive(Debug, thiserror::Error)]
pub enum StateStoreError {
    #[error("Non existent shard: {shard:?}")]
    NonExistentShard { shard: Vec<u8> },
    #[error("Encoding error: {0}")]
    EncodingError(#[from] io::Error),
    #[error(transparent)]
    Custom(#[from] anyhow::Error),
    #[error("Error: {0}")]
    CustomStr(String),
    #[error("{kind} not found with id {id}")]
    NotFound { kind: &'static str, id: String },
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
