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

use std::{collections::HashMap, fmt::Display};

use serde::{Deserialize, Serialize};
use tari_template_lib::prelude::ComponentAddress;

use crate::serde_with;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Event {
    #[serde(with = "serde_with::hex")]
    component_address: ComponentAddress,
    topic: String,
    payload: HashMap<String, String>,
}

impl Event {
    pub fn new(component_address: ComponentAddress, topic: String) -> Self {
        Self::new_with_payload(component_address, topic, HashMap::new())
    }

    pub fn new_with_payload(
        component_address: ComponentAddress,
        topic: String,
        payload: HashMap<String, String>,
    ) -> Self {
        Self {
            component_address,
            topic,
            payload,
        }
    }

    pub fn component_address(&self) -> ComponentAddress {
        self.component_address
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

    pub fn get_full_payload(&self) -> HashMap<String, String> {
        self.payload.clone()
    }
}

impl Display for Event {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "event: component_address {}, topic {}",
            self.component_address, self.topic
        )
    }
}
