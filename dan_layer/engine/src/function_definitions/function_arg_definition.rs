// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use serde::{Deserialize, Serialize};
use tari_template_abi::Type;

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct FunctionArgDefinition {
    pub name: String,
    #[serde(rename = "type")]
    pub arg_type: ArgType,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "snake_case")]
pub enum ArgType {
    String,
}

impl ArgType {
    pub fn to_type(&self) -> Type {
        match self {
            ArgType::String => Type::String,
        }
    }
}
