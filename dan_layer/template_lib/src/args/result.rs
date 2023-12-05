//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tari_bor::{decode_exact, from_value, to_value, BorError};

/// The result of an instruction invocation, which is either the CBOR encoded result value or a `String` with an error message
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvokeResult(Result<tari_bor::Value, String>);

impl InvokeResult {
    pub fn from_value(value: tari_bor::Value) -> Self {
        Self(Ok(value))
    }

    pub fn encode<T: Serialize + ?Sized>(output: &T) -> Result<Self, BorError> {
        let output = to_value(output)?;
        Ok(Self(Ok(output)))
    }

    pub fn decode<T: DeserializeOwned>(self) -> Result<T, BorError> {
        match self.0 {
            Ok(output) => from_value(&output),
            Err(err) => Err(BorError::new(err)),
        }
    }

    pub fn into_value(self) -> Result<tari_bor::Value, BorError> {
        self.0.map_err(BorError::new)
    }

    pub fn raw(data: Vec<u8>) -> Self {
        // TODO: unwrap
        Self(Ok(decode_exact(&data).unwrap()))
    }

    pub fn unit() -> Self {
        Self(Ok(to_value(&()).unwrap()))
    }
}
