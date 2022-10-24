//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::collections::BTreeMap;

use tari_dan_common_types::ShardId;
use tari_template_abi::{decode, encode, Decode, Encode};
use tari_template_lib::{models::ComponentInstance, Hash};

use crate::{models::Resource, runtime::logs::LogEntry, wasm::ExecutionResult};

#[derive(Debug)]
pub struct FinalizeResult {
    pub transaction_hash: Hash,
    pub logs: Vec<LogEntry>,
    pub execution_results: Vec<ExecutionResult>,
    pub result: TransactionResult,
}

impl FinalizeResult {
    pub fn new(transaction_hash: Hash, logs: Vec<LogEntry>, result: TransactionResult) -> Self {
        Self {
            transaction_hash,
            logs,
            execution_results: Vec::new(),
            result,
        }
    }
}

#[derive(Debug, Clone)]
pub enum TransactionResult {
    Accept(SubstateDiff),
    Reject(RejectResult),
}

impl TransactionResult {
    pub fn expect(self, msg: &str) -> SubstateDiff {
        match self {
            Self::Accept(diff) => diff,
            Self::Reject(result) => panic!("{}. Transaction was rejected {}", msg, result.reason),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SubstateDiff {
    up_substates: BTreeMap<ShardId, SubstateValue>,
    down_substates: Vec<ShardId>,
}

impl SubstateDiff {
    pub fn new() -> Self {
        Self {
            up_substates: BTreeMap::new(),
            down_substates: Vec::new(),
        }
    }

    pub fn up<T: Into<ShardId>>(&mut self, shard_id: T, value: SubstateValue) {
        self.up_substates.insert(shard_id.into(), value);
    }

    pub fn down<T: Into<ShardId>>(&mut self, shard_id: T) {
        self.down_substates.push(shard_id.into());
    }

    pub fn up_iter(&self) -> impl Iterator<Item = (&ShardId, &SubstateValue)> + '_ {
        self.up_substates.iter()
    }

    pub fn down_iter(&self) -> impl Iterator<Item = &ShardId> + '_ {
        self.down_substates.iter()
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub struct SubstateValue {
    substate: Substate,
    version: u32,
}

impl SubstateValue {
    pub fn new<T: Into<Substate>>(substate: T) -> Self {
        Self {
            substate: substate.into(),
            version: 0,
        }
    }

    pub fn into_substate(self) -> Substate {
        self.substate
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        encode(self).unwrap()
    }

    pub fn from_bytes(bytes: &[u8]) -> std::io::Result<Self> {
        decode(bytes)
    }
}

#[derive(Debug, Clone, Encode, Decode)]
pub enum Substate {
    Component(ComponentInstance),
    Resource(Resource),
}

impl From<ComponentInstance> for Substate {
    fn from(component: ComponentInstance) -> Self {
        Self::Component(component)
    }
}

impl From<Resource> for Substate {
    fn from(resource: Resource) -> Self {
        Self::Resource(resource)
    }
}

#[derive(Debug, Clone, Encode)]
pub struct RejectResult {
    // TODO: This should contain data required for a rejection vote
    pub reason: String,
}
