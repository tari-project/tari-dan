//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(not(feature = "std"))]
extern crate alloc;

#[cfg(not(feature = "std"))]
use alloc::{format, vec::Vec};

pub mod adapters;
mod error;
mod macros;
mod raw;
#[cfg(feature = "serde")]
pub mod serde_codec;
mod tag;
mod value;
#[cfg(feature = "serde")]
mod value_serde;
mod walker;

mod byte_counter;

pub use byte_counter::ByteCounter;
pub use error::BorError;
pub use macros::__cbor_macro;
pub use minicbor::{self, CborLen, Decode, Encode};
pub use raw::RawCbor;
#[cfg(feature = "serde")]
pub use serde::{self, Deserialize, Serialize, de::DeserializeOwned};
pub use tag::*;
pub use value::{MAX_DECODE_DEPTH, Value};
pub use walker::*;

/// Encode a value into a freshly allocated `Vec<u8>` using the unit context.
pub fn encode<T: Encode<()> + ?Sized>(val: &T) -> Result<Vec<u8>, BorError> {
    encode_with(val, &mut ())
}

/// Encode a value into a freshly allocated `Vec<u8>` using a user-provided context.
pub fn encode_with<C, T: Encode<C> + ?Sized>(val: &T, ctx: &mut C) -> Result<Vec<u8>, BorError> {
    minicbor::to_vec_with(val, ctx).map_err(BorError::from)
}

/// Encode a value into a [`std::io::Write`] sink using the unit context (std feature).
#[cfg(feature = "std")]
pub fn encode_into_writer<T, W>(val: &T, writer: &mut W) -> Result<(), BorError>
where
    T: Encode<()> + ?Sized,
    W: std::io::Write,
{
    encode_into_writer_with(val, writer, &mut ())
}

/// Encode a value into a [`std::io::Write`] sink using a user-provided context (std feature).
#[cfg(feature = "std")]
pub fn encode_into_writer_with<C, T, W>(val: &T, writer: &mut W, ctx: &mut C) -> Result<(), BorError>
where
    T: Encode<C> + ?Sized,
    W: std::io::Write,
{
    let writer = minicbor::encode::write::Writer::new(writer);
    minicbor::encode_with(val, writer, ctx).map_err(BorError::from)
}

/// Encode a value into a [`minicbor::encode::Write`] sink using the unit context (no-std).
#[cfg(not(feature = "std"))]
pub fn encode_into_writer<T, W>(val: &T, writer: W) -> Result<(), BorError>
where
    T: Encode<()> + ?Sized,
    W: minicbor::encode::Write,
    W::Error: core::fmt::Display,
{
    encode_into_writer_with(val, writer, &mut ())
}

/// Encode a value into a [`minicbor::encode::Write`] sink using a user-provided context (no-std).
#[cfg(not(feature = "std"))]
pub fn encode_into_writer_with<C, T, W>(val: &T, writer: W, ctx: &mut C) -> Result<(), BorError>
where
    T: Encode<C> + ?Sized,
    W: minicbor::encode::Write,
    W::Error: core::fmt::Display,
{
    minicbor::encode_with(val, writer, ctx).map_err(BorError::from)
}

/// Pre-calculate the encoded length in bytes of a value via [`minicbor::CborLen`] (unit context).
///
/// Types should `#[derive(CborLen)]` alongside `Encode`/`Decode` so this is O(1) over
/// the type structure. The fallback path through [`ByteCounter`] is still available for
/// types that haven't derived `CborLen` yet (see [`encoded_len_via_writer`]).
///
/// The `Result` return type is preserved for API compatibility — this function cannot
/// actually fail today.
pub fn encoded_len<T: CborLen<()> + ?Sized>(val: &T) -> Result<usize, BorError> {
    encoded_len_with(val, &mut ())
}

/// Pre-calculate the encoded length in bytes via [`minicbor::CborLen`] using a user-provided context.
pub fn encoded_len_with<C, T: CborLen<C> + ?Sized>(val: &T, ctx: &mut C) -> Result<usize, BorError> {
    Ok(minicbor::len_with(val, ctx))
}

/// Pre-calculate the encoded length in bytes (unit context), returning an error if it exceeds `limit`.
pub fn encoded_len_with_limit<T: CborLen<()> + ?Sized>(val: &T, limit: usize) -> Result<usize, BorError> {
    encoded_len_with_limit_with(val, limit, &mut ())
}

/// Pre-calculate the encoded length in bytes using a user-provided context, returning an error if it exceeds `limit`.
pub fn encoded_len_with_limit_with<C, T: CborLen<C> + ?Sized>(
    val: &T,
    limit: usize,
    ctx: &mut C,
) -> Result<usize, BorError> {
    let n = minicbor::len_with(val, ctx);
    if n > limit {
        return Err(BorError::new(format!("encoded length {n} exceeds limit {limit}")));
    }
    Ok(n)
}

/// Fallback length calculation that drives the encoder. Use this for types that haven't
/// derived [`CborLen`] yet (during the in-progress migration). Prefer [`encoded_len`].
pub fn encoded_len_via_writer<T: Encode<()> + ?Sized>(val: &T) -> Result<usize, BorError> {
    let mut counter = ByteCounter::new();
    minicbor::encode(val, &mut counter).map_err(|e| BorError::new(format!("encoded_len failed: {e}")))?;
    Ok(counter.get())
}

/// Encode a value into a dynamic [`Value`] tree (unit context).
pub fn to_value<T: Encode<()> + ?Sized>(val: &T) -> Result<Value, BorError> {
    let bytes = encode(val)?;
    decode(&bytes)
}

/// Decode a value out of a dynamic [`Value`] tree by re-encoding to bytes and decoding via the
/// target type. Useful for tests and dynamic conversion; for production paths prefer to
/// `decode` the original bytes directly.
pub fn from_value<T: for<'b> Decode<'b, ()>>(val: &Value) -> Result<T, BorError> {
    let bytes = encode(val)?;
    decode(&bytes)
}

/// Decode a single value from a byte slice (unit context). Extra trailing bytes are ignored.
pub fn decode<T: for<'b> Decode<'b, ()>>(input: &[u8]) -> Result<T, BorError> {
    minicbor::decode(input).map_err(BorError::from)
}

/// Decode a single value from a byte slice using a user-provided context. Extra trailing bytes are ignored.
pub fn decode_with<C, T>(input: &[u8], ctx: &mut C) -> Result<T, BorError>
where T: for<'b> Decode<'b, C> {
    minicbor::decode_with(input, ctx).map_err(BorError::from)
}

/// Decode a single value from a byte slice (unit context). Returns an error if any bytes remain after
/// decoding.
pub fn decode_exact<T: for<'b> Decode<'b, ()>>(input: &[u8]) -> Result<T, BorError> {
    decode_exact_with(input, &mut ())
}

/// Decode a single value from a byte slice using a user-provided context. Returns an error if any bytes
/// remain after decoding.
pub fn decode_exact_with<C, T>(input: &[u8], ctx: &mut C) -> Result<T, BorError>
where T: for<'b> Decode<'b, C> {
    let mut d = minicbor::Decoder::new(input);
    let value = d.decode_with(ctx).map_err(BorError::from)?;
    let consumed = d.position();
    if consumed != input.len() {
        return Err(BorError::new(format!(
            "decode_exact: {} bytes remaining on input",
            input.len() - consumed
        )));
    }
    Ok(value)
}
