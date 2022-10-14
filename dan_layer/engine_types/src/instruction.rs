//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_abi::Encode;
use tari_template_lib::{
    args::Arg,
    models::{ComponentAddress, TemplateAddress},
    Hash,
};

use crate::hashing::hasher;

#[derive(Debug, Clone, Encode, Deserialize, Serialize)]
#[serde(tag = "type")]
pub enum Instruction {
    CallFunction {
        template_address: TemplateAddress,
        function: String,
        args: Vec<Arg>,
    },
    CallMethod {
        template_address: TemplateAddress,
        component_address: ComponentAddress,
        method: String,
        args: Vec<Arg>,
    },
    PutLastInstructionOutputOnWorkspace {
        key: Vec<u8>,
    },
}

impl Instruction {
    pub fn hash(&self) -> Hash {
        hasher("instruction").chain(self).result()
    }
}
