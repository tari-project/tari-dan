//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
use alloc::{format, string::ToString, vec::Vec};

#[cfg(not(feature = "std"))]
extern crate alloc;

mod tag;
pub use tag::*;

mod error;
#[cfg(feature = "json_encoding")]
pub mod json_encoding;
mod walker;

pub use ciborium::{cbor, value::Value};
use ciborium::{de::from_reader, ser::into_writer};
pub use error::BorError;
pub use serde::{self, de::DeserializeOwned, Deserialize, Serialize};
pub use walker::*;

pub fn encode_with_len<T: Serialize>(val: &T) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
    buf.extend([0u8; 4]);

    encode_into(val, &mut buf).expect("Vec<u8> Write impl is infallible");

    let len = ((buf.len() - 4) as u32).to_le_bytes();
    buf[..4].copy_from_slice(&len);

    buf
}

#[cfg(not(feature = "std"))]
pub fn encode_into<T, W>(val: &T, writer: &mut W) -> Result<(), BorError>
where
    W::Error: core::fmt::Debug,
    T: Serialize + ?Sized,
    W: ciborium_io::Write,
{
    into_writer(&val, writer).map_err(to_bor_error)
}

#[cfg(feature = "std")]
pub fn encode_into<T: Serialize + ?Sized, W: std::io::Write>(val: &T, writer: &mut W) -> Result<(), BorError> {
    into_writer(&val, writer).map_err(to_bor_error)
}

pub fn encode<T: Serialize + ?Sized>(val: &T) -> Result<Vec<u8>, BorError> {
    let mut buf = Vec::with_capacity(512);
    encode_into(val, &mut buf).map_err(|e| BorError::new(format!("{:?}", e)))?;
    Ok(buf)
}

/// Encodes any Rust type using CBOR
pub fn to_value<T: Serialize + ?Sized>(val: &T) -> Result<Value, BorError> {
    Value::serialized(val).map_err(to_bor_error)
}

pub fn from_value<T: DeserializeOwned>(val: &Value) -> Result<T, BorError> {
    Value::deserialized(val).map_err(to_bor_error)
}

pub fn decode<T: DeserializeOwned>(mut input: &[u8]) -> Result<T, BorError> {
    decode_inner(&mut input)
}

fn decode_inner<T: DeserializeOwned>(input: &mut &[u8]) -> Result<T, BorError> {
    let result = from_reader(input).map_err(to_bor_error)?;
    Ok(result)
}

pub fn decode_exact<T: DeserializeOwned>(mut input: &[u8]) -> Result<T, BorError> {
    let val = decode_inner(&mut input)?;
    if !input.is_empty() {
        return Err(BorError::new(format!(
            "decode_exact: {} bytes remaining on input",
            input.len()
        )));
    }
    Ok(val)
}

pub fn decode_len(input: &[u8]) -> Result<usize, BorError> {
    if input.len() < 4 {
        return Err(BorError::new("Not enough bytes to decode length".to_string()));
    }

    let mut buf = [0u8; 4];
    buf.copy_from_slice(&input[..4]);
    let len = u32::from_le_bytes(buf);
    Ok(len as usize)
}

fn to_bor_error<E>(e: E) -> BorError
where E: core::fmt::Display {
    BorError::new(e.to_string())
}
