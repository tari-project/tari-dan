//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{any::type_name, fmt, rc::Rc};

use serde::de::DeserializeOwned;
use tari_bor::{BorError, decode_exact, decode_exact_with_max_depth};
use tari_engine_types::limits;
use tari_template_lib::types::bytes::Bytes;

use crate::runtime::{Runtime, RuntimeError, RuntimeInterface};

/// Why an engine call argument could not be charged for.
#[derive(Debug, thiserror::Error)]
pub enum EngineArgChargeError {
    /// The argument is not CBOR the engine can read, or nests deeper than it accepts.
    #[error(transparent)]
    Unreadable(BorError),
    /// The transaction cannot cover it.
    #[error(transparent)]
    Runtime(#[from] RuntimeError),
}

#[derive(Clone, Default)]
pub struct EngineArgs {
    args: Vec<Bytes>,
    /// Charges for each argument before it is decoded. Set on the arguments of an engine call, which
    /// a template wrote, and unset on arguments the engine built itself.
    metered_by: Option<Rc<dyn RuntimeInterface>>,
}

impl EngineArgs {
    pub fn new() -> Self {
        Self::default()
    }

    /// The arguments of an engine call, each charged to `runtime` before it is decoded.
    pub fn metered(args: Vec<Bytes>, runtime: &Runtime) -> Self {
        Self {
            args,
            metered_by: Some(runtime.shared_interface()),
        }
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
        let Some(arg) = self.args.get(index) else {
            return Ok(None);
        };
        let decode_failed = |e: BorError| RuntimeError::InvalidArgument {
            argument: type_name::<T>(),
            reason: format!("Argument failed to decode. Err: {:?}", e),
        };
        let decoded = match &self.metered_by {
            // The charge walks the argument for its nesting depth on the way.
            Some(interface) => {
                interface.charge_engine_arg(arg).map_err(|e| match e {
                    EngineArgChargeError::Unreadable(e) => decode_failed(e),
                    EngineArgChargeError::Runtime(e) => e,
                })?;
                decode_exact(arg)
            },
            // An argument is an opaque blob inside the transaction, so the bound the transaction
            // was decoded under did not reach into it.
            None => decode_exact_with_max_depth(arg, limits::MAX_CBOR_NESTING_DEPTH),
        };
        decoded.map(Some).map_err(decode_failed)
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

impl fmt::Debug for EngineArgs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineArgs")
            .field("args", &self.args)
            .field("metered", &self.metered_by.is_some())
            .finish()
    }
}

impl From<Vec<Bytes>> for EngineArgs {
    fn from(args: Vec<Bytes>) -> Self {
        Self { args, metered_by: None }
    }
}

impl From<Bytes> for EngineArgs {
    fn from(arg: Bytes) -> Self {
        Self::from(vec![arg])
    }
}
