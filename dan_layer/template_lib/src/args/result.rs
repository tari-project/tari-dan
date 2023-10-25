//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tari_bor::{decode_exact, encode, BorError};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InvokeResult(Result<Vec<u8>, String>);

impl InvokeResult {
    pub fn encode<T: Serialize + ?Sized>(output: &T) -> Result<Self, BorError> {
        let output = encode(output)?;
        Ok(Self(Ok(output)))
    }

    pub fn decode<T: DeserializeOwned>(self) -> Result<T, BorError> {
        match self.0 {
            Ok(output) => decode_exact(&output),
            Err(err) => Err(BorError::new(err)),
        }
    }

    pub fn raw(data: Vec<u8>) -> Self {
        Self(Ok(data))
    }

    pub fn unit() -> Self {
        Self(Ok(encode(&()).unwrap()))
    }
}
