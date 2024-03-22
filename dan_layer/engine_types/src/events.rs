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

use std::fmt::Display;

use serde::{Deserialize, Serialize};
use tari_template_lib::{
    models::{Metadata, TemplateAddress},
    Hash,
};
#[cfg(feature = "ts")]
use ts_rs::TS;

use crate::{serde_with, substate::SubstateId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "ts", derive(TS), ts(export, export_to = "../../bindings/src/types/"))]
pub struct Event {
    #[cfg_attr(feature = "ts", ts(type = "string | null"))]
    substate_id: Option<SubstateId>,
    #[serde(with = "serde_with::hex")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    template_address: TemplateAddress,
    #[serde(with = "serde_with::hex")]
    #[cfg_attr(feature = "ts", ts(type = "string"))]
    tx_hash: Hash,
    topic: String,
    // NOTE: We need to use an ordered map here. HashMaps are unordered, so when we pledge this state the hash
    // resulting hash may differ.
    payload: Metadata,
}

impl Event {
    pub fn new(
        substate_id: Option<SubstateId>,
        template_address: TemplateAddress,
        tx_hash: Hash,
        topic: String,
        payload: Metadata,
    ) -> Self {
        Self {
            substate_id,
            template_address,
            tx_hash,
            topic,
            payload,
        }
    }

    pub fn substate_id(&self) -> Option<SubstateId> {
        self.substate_id.clone()
    }

    pub fn template_address(&self) -> TemplateAddress {
        self.template_address
    }

    pub fn tx_hash(&self) -> Hash {
        self.tx_hash
    }

    pub fn topic(&self) -> String {
        self.topic.clone()
    }

    pub fn add_payload(&mut self, key: String, value: String) {
        self.payload.insert(key, value);
    }

    pub fn get_payload(&self, key: &str) -> Option<String> {
        self.payload.get(key).cloned()
    }

    pub fn payload(&self) -> &Metadata {
        &self.payload
    }

    pub fn into_payload(self) -> Metadata {
        self.payload
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "event: substate_id {:?}, template_address {}, tx_hash {}, topic {} and payload {:?}",
            self.substate_id, self.template_address, self.tx_hash, self.topic, self.payload
        )
    }
}
