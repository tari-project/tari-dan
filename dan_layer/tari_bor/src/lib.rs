//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
#![cfg_attr(not(feature = "std"), no_std)]

use borsh::maybestd::{format, io, vec::Vec};
// This is to make the borsh macros happy
pub use borsh::{self, BorshDeserialize as Decode, BorshSerialize as Encode};

pub fn encode_with_len<T: Encode>(val: &T) -> Vec<u8> {
    let mut buf = Vec::with_capacity(512);
    buf.extend([0u8; 4]);

    encode_into(val, &mut buf).expect("Vec<u8> Write impl is infallible");

    let len = ((buf.len() - 4) as u32).to_le_bytes();
    buf[..4].copy_from_slice(&len);

    buf
}

pub fn encode_into<T: Encode + ?Sized, W: io::Write>(val: &T, writer: &mut W) -> io::Result<()> {
    val.serialize(writer)
}

pub fn encode<T: Encode + ?Sized>(val: &T) -> io::Result<Vec<u8>> {
    let mut buf = Vec::with_capacity(512);
    encode_into(val, &mut buf)?;
    Ok(buf)
}

pub fn decode<T: Decode>(mut input: &[u8]) -> io::Result<T> {
    decode_inner(&mut input)
}

fn decode_inner<T: Decode>(input: &mut &[u8]) -> io::Result<T> {
    let result = T::deserialize(input)?;
    Ok(result)
}

pub fn decode_exact<T: Decode>(mut input: &[u8]) -> io::Result<T> {
    let val = decode_inner(&mut input)?;
    if !input.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::Other,
            format!("decode_exact: {} bytes remaining on input", input.len()),
        ));
    }
    Ok(val)
}

pub fn decode_len(input: &[u8]) -> io::Result<usize> {
    if input.len() < 4 {
        return Err(io::Error::new(
            io::ErrorKind::UnexpectedEof,
            "Not enough bytes to decode length",
        ));
    }

    let mut buf = [0u8; 4];
    buf.copy_from_slice(&input[..4]);
    let len = u32::from_le_bytes(buf);
    Ok(len as usize)
}
