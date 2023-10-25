//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::any::type_name;

use serde::de::DeserializeOwned;
use tari_bor::decode_exact;

use crate::runtime::RuntimeError;

#[derive(Debug, Clone, Default)]
pub struct EngineArgs {
    args: Vec<Vec<u8>>,
}

impl EngineArgs {
    pub fn new() -> Self {
        Self { args: Vec::new() }
    }

    pub fn get<T: DeserializeOwned>(&self, index: usize) -> Result<T, RuntimeError> {
        self.args
            .get(index)
            .map(|arg| decode_exact(arg))
            .transpose()
            .map_err(|e| RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: format!("Argument failed to decode. Err: {:?}", e),
            })?
            .ok_or_else(|| RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: "Argument not provided".to_string(),
            })
    }

    pub fn assert_one_arg<T: DeserializeOwned>(&self) -> Result<T, RuntimeError> {
        if self.len() == 1 {
            self.get(0)
        } else {
            Err(RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: format!("Expected only one argument but got {}", self.len()),
            })
        }
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    pub fn assert_is_empty(&self, op_name: &'static str) -> Result<(), RuntimeError> {
        if self.is_empty() {
            Ok(())
        } else {
            Err(RuntimeError::InvalidArgument {
                argument: op_name,
                reason: format!("Expected no arguments but got {}", self.len()),
            })
        }
    }
}

impl From<Vec<Vec<u8>>> for EngineArgs {
    fn from(args: Vec<Vec<u8>>) -> Self {
        Self { args }
    }
}
