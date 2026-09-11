//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::any::type_name;

use serde::de::DeserializeOwned;
use tari_bor::decode_exact;
use tari_template_lib::types::bytes::Bytes;

use crate::runtime::RuntimeError;

#[derive(Debug, Clone, Default)]
pub struct EngineArgs {
    args: Vec<Bytes>,
}

impl EngineArgs {
    pub fn new() -> Self {
        Self { args: Vec::new() }
    }

    pub fn get<T>(&self, index: usize) -> Result<T, RuntimeError>
    where T: DeserializeOwned + for<'b> tari_bor::Decode<'b, ()> {
        self.get_opt(index)?.ok_or_else(|| RuntimeError::InvalidArgument {
            argument: type_name::<T>(),
            reason: "Argument not provided".to_string(),
        })
    }

    pub fn get_opt<T>(&self, index: usize) -> Result<Option<T>, RuntimeError>
    where T: DeserializeOwned + for<'b> tari_bor::Decode<'b, ()> {
        self.args
            .get(index)
            .map(|arg| decode_exact(arg))
            .transpose()
            .map_err(|e| RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: format!("Argument failed to decode. Err: {:?}", e),
            })
    }

    /// Total encoded size of every argument. Available without decoding, so a price derived from it
    /// can be charged before the work — and before the arguments are read — is done.
    pub fn encoded_len(&self) -> u64 {
        self.args.iter().map(|arg| arg.len() as u64).sum()
    }

    /// Encoded size of one argument, or zero if there is no argument at `index`. Available without decoding, for
    /// the same reason as [`Self::encoded_len`].
    pub fn arg_encoded_len(&self, index: usize) -> u64 {
        self.args.get(index).map_or(0, |arg| arg.len() as u64)
    }

    pub fn assert_one_arg<T>(&self) -> Result<T, RuntimeError>
    where T: DeserializeOwned + for<'b> tari_bor::Decode<'b, ()> {
        if self.len() == 1 {
            self.get(0)
        } else {
            Err(RuntimeError::InvalidArgument {
                argument: type_name::<T>(),
                reason: format!("Expected only one argument but got {}", self.len()),
            })
        }
    }

    pub fn assert_n_args(&self, n: usize) -> Result<(), RuntimeError> {
        if self.len() != n {
            return Err(RuntimeError::InvalidNumberOfArguments {
                expected: n,
                len: self.len(),
            });
        }
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.args.len()
    }

    pub fn is_empty(&self) -> bool {
        self.args.is_empty()
    }

    pub fn assert_no_args(&self, op_name: &'static str) -> Result<(), RuntimeError> {
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

impl From<Vec<Bytes>> for EngineArgs {
    fn from(args: Vec<Bytes>) -> Self {
        Self { args }
    }
}

impl From<Bytes> for EngineArgs {
    fn from(arg: Bytes) -> Self {
        Self { args: vec![arg] }
    }
}
