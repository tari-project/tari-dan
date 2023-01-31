//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_bor::{borsh, decode_exact, encode, Decode, Encode};
use tari_template_abi::rust::io;

#[derive(Clone, Debug, Decode, Encode)]
pub struct InvokeResult(Result<Vec<u8>, String>);

impl InvokeResult {
    pub fn encode<T: Encode + ?Sized>(output: &T) -> io::Result<Self> {
        let output = encode(output)?;
        Ok(Self(Ok(output)))
    }

    pub fn decode<T: Decode>(self) -> io::Result<T> {
        match self.0 {
            Ok(output) => decode_exact(&output),
            Err(err) => Err(io::Error::new(io::ErrorKind::Other, err)),
        }
    }

    pub fn raw(data: Vec<u8>) -> Self {
        Self(Ok(data))
    }

    pub fn unwrap_decode<T: Decode>(self) -> T {
        self.decode().unwrap()
    }

    pub fn unit() -> Self {
        Self(Ok(encode(&()).unwrap()))
    }
}
