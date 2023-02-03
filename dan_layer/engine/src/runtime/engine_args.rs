//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::any::type_name;

use tari_bor::{borsh, decode_exact, Decode, Encode};

use crate::runtime::RuntimeError;

#[derive(Debug, Clone, Encode, Decode, Default)]
pub struct EngineArgs {
    args: Vec<Vec<u8>>,
}

impl EngineArgs {
    pub fn new() -> Self {
        Self { args: Vec::new() }
    }

    pub fn get<T: Decode>(&self, index: usize) -> Result<T, RuntimeError> {
        self.args
            .get(index)
            .map(|arg| decode_exact(arg))
            .transpose()
            .map_err(|e| RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: format!("Argument failed to decode. Err: {}", e),
            })?
            .ok_or_else(|| RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: "Argument not provided".to_string(),
            })
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }
}

impl From<Vec<Vec<u8>>> for EngineArgs {
    fn from(args: Vec<Vec<u8>>) -> Self {
        Self { args }
    }
}
