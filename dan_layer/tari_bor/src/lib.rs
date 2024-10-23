//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
use alloc::{fmt, format, string::ToString, vec::Vec};

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(feature = "std")]
use std::fmt;

mod tag;
pub use tag::*;

mod error;
#[cfg(feature = "json_encoding")]
pub mod json_encoding;
mod walker;

pub use ciborium::{cbor, value::Value};
use ciborium::{de::from_reader, ser::into_writer};
pub use ciborium_io::{Read, Write};
pub use error::BorError;
pub use serde::{self, de::DeserializeOwned, Deserialize, Serialize};
pub use walker::*;

pub fn encode_with_len<T: Serialize>(val: &T) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
    encode_with_len_to_writer(&mut buf, val).expect("Vec<u8> Write impl is infallible");
    // buf.extend([0u8; 4]);

    // encode_into_writer(val, &mut buf).expect("Vec<u8> Write impl is infallible");

    // let len = ((buf.len() - 4) as u32).to_le_bytes();
    // buf[..4].copy_from_slice(&len);

    buf
}

pub fn encode_with_len_to_writer<T, W>(mut writer: W, val: &T) -> Result<(), BorError>
where
    T: Serialize,
    W: Write,
    W::Error: fmt::Debug,
{
    let len = encoded_len(val)?;
    writer
        .write_all(&(len as u32).to_le_bytes())
        .map_err(|e| BorError::new(format!("{:?}", e)))?;
    encode_into_writer(val, writer)?;
    Ok(())
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
pub fn encode_into_std_writer<T: Serialize + ?Sized, W: std::io::Write>(
    val: &T,
    writer: &mut W,
) -> Result<(), BorError> {
    into_writer(&val, writer).map_err(to_bor_error)
}

pub fn encode_into_writer<T, W>(val: &T, writer: W) -> Result<(), BorError>
where
    T: Serialize + ?Sized,
    W: Write,
    W::Error: fmt::Debug,
{
    into_writer(&val, writer).map_err(to_bor_error)
}

pub fn encode<T: Serialize + ?Sized>(val: &T) -> Result<Vec<u8>, BorError> {
    let mut buf = Vec::with_capacity(512);
    encode_into_writer(val, &mut buf).map_err(|e| BorError::new(format!("{:?}", e)))?;
    Ok(buf)
}

pub fn encoded_len<T: Serialize + ?Sized>(val: &T) -> Result<usize, BorError> {
    let mut counter = ByteCounter::new();
    encode_into_writer(val, &mut counter).map_err(|e| BorError::new(format!("{:?}", e)))?;
    Ok(counter.get())
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

pub fn decode_from_reader<T, R>(reader: R) -> Result<T, BorError>
where
    T: DeserializeOwned,
    R: Read,
    R::Error: fmt::Debug,
{
    let result = from_reader(reader).map_err(to_bor_error)?;
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
where E: fmt::Display {
    BorError::new(e.to_string())
}

#[derive(Debug, Clone, Default)]
pub struct ByteCounter {
    count: usize,
}

impl ByteCounter {
    pub fn new() -> Self {
        Default::default()
    }

    pub fn get(&self) -> usize {
        self.count
    }
}

#[derive(Debug)]
pub struct ByteCounterError;

impl Write for &mut ByteCounter {
    type Error = ByteCounterError;

    fn write_all(&mut self, data: &[u8]) -> Result<(), Self::Error> {
        let len = data.len();
        self.count += len;
        Ok(())
    }

    fn flush(&mut self) -> Result<(), Self::Error> {
        Ok(())
    }
}
