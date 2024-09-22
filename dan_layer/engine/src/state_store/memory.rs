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
use std::{collections::HashMap, sync::Arc};

use tari_engine_types::substate::{Substate, SubstateId};

use crate::state_store::{StateReader, StateStoreError, StateWriter};

#[derive(Debug, Clone, Default)]
pub struct MemoryStateStore {
    state: HashMap<SubstateId, Substate>,
}

impl MemoryStateStore {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub fn set_many<T: IntoIterator<Item = (SubstateId, Substate)>>(&mut self, iter: T) -> Result<(), StateStoreError> {
        self.state.extend(iter);
        Ok(())
    }

    pub fn count(&self) -> usize {
        self.state.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SubstateId, &Substate)> {
        self.state.iter()
    }

    pub fn delete_state(&mut self, key: &SubstateId) {
        self.state.remove(key);
    }

    pub fn into_read_only(self) -> ReadOnlyMemoryStateStore {
        ReadOnlyMemoryStateStore {
            state: Arc::new(self.state),
        }
    }
}

impl StateReader for MemoryStateStore {
    fn get_state(&self, key: &SubstateId) -> Result<&Substate, StateStoreError> {
        self.state.get(key).ok_or_else(|| StateStoreError::NotFound {
            kind: "state",
            key: key.to_string(),
        })
    }

    fn exists(&self, key: &SubstateId) -> Result<bool, StateStoreError> {
        Ok(self.state.contains_key(key))
    }
}

impl StateWriter for MemoryStateStore {
    fn set_state(&mut self, key: SubstateId, value: Substate) -> Result<(), StateStoreError> {
        self.state.insert(key, value);
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReadOnlyMemoryStateStore {
    state: Arc<HashMap<SubstateId, Substate>>,
}

impl ReadOnlyMemoryStateStore {
    pub fn count(&self) -> usize {
        self.state.len()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&SubstateId, &Substate)> {
        self.state.iter()
    }
}

impl StateReader for ReadOnlyMemoryStateStore {
    fn get_state(&self, key: &SubstateId) -> Result<&Substate, StateStoreError> {
        self.state.get(key).ok_or_else(|| StateStoreError::NotFound {
            kind: "state",
            key: key.to_string(),
        })
    }

    fn exists(&self, key: &SubstateId) -> Result<bool, StateStoreError> {
        Ok(self.state.contains_key(key))
    }
}
