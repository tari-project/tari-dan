//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::fmt::{Display, Formatter};

use tari_template_lib::auth::NativeFunctionCall;

#[derive(Debug, Clone)]
pub enum FunctionIdent {
    Native(NativeFunctionCall),
    Template { module_name: String, function: String },
}

impl Display for FunctionIdent {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            FunctionIdent::Native(native_fn) => write!(f, "native.{}", native_fn),
            FunctionIdent::Template { module_name, function } => write!(f, "template.{}.{}", module_name, function),
        }
    }
}
