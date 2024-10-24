//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

#[cfg(target_arch = "wasm32")]
mod wasm;
use serde::{de::DeserializeOwned, Serialize};
#[cfg(target_arch = "wasm32")]
pub use wasm::*;
#[cfg(not(target_arch = "wasm32"))]
mod non_wasm;
#[cfg(not(target_arch = "wasm32"))]
pub use non_wasm::*;
use tari_bor::{decode_exact, decode_len, encode_into_writer, encoded_len};

use crate::{
    ops::EngineOp,
    rust::{fmt, mem, ptr::copy, slice, vec::Vec},
};

pub fn wrap_ptr(mut v: Vec<u8>) -> *mut u8 {
    let ptr = v.as_mut_ptr();
    mem::forget(v);
    ptr
}

pub fn call_engine<T: Serialize + fmt::Debug, U: DeserializeOwned>(op: EngineOp, input: &T) -> U {
    let len = encoded_len(&input).unwrap();
    let mut encoded = Vec::with_capacity(len);
    encode_into_writer(input, &mut encoded).unwrap();
    let len = encoded.len();
    let input_ptr = wrap_ptr(encoded) as *const _;
    let ptr = unsafe { tari_engine(op.as_i32(), input_ptr, len) };
    if ptr.is_null() {
        panic!("Engine call returned null for op {:?}", op);
    }
    let slice = unsafe { slice::from_raw_parts(ptr as *const _, 4) };
    let len = decode_len(slice).unwrap();
    // Take ownership of the data and deallocate it at the end of the function
    let data = unsafe { Vec::from_raw_parts(ptr, len + 4, len + 4) };
    decode_exact(&data[4..4 + len]).unwrap_or_else(|e| {
        panic!(
            "Failed to decode response from engine for op {:?} with input: {:?}: {:?}",
            op, input, e,
        )
    })
}

/// Requests the engine to write debug data
pub fn call_debug<T: AsRef<[u8]>>(data: T) {
    let ptr = data.as_ref().as_ptr();
    let len = data.as_ref().len();
    unsafe { debug(ptr, len) }
}

/// Allocates a block of memory of length `len` bytes.
#[no_mangle]
pub extern "C" fn tari_alloc(len: u32) -> *mut u8 {
    let cap = (len + 4) as usize;
    let mut buf = Vec::<u8>::with_capacity(cap);
    let ptr = buf.as_mut_ptr();
    mem::forget(buf);
    unsafe {
        copy(len.to_le_bytes().as_ptr(), ptr, 4);
    }
    ptr
}

// This is currently not needed as every engine alloc should be freed by the WASM template.
// Note there is no appropriate way to force this behaviour but since WASM is already sandboxed, any leaked memory is
// released after execution in any case.
//
// /// Frees a block of memory allocated by `tari_alloc`.
// ///
// /// # Safety
// /// Caller must ensure that ptr must be a valid pointer to a block of memory allocated by `tari_alloc`.
// #[no_mangle]
// pub unsafe extern "C" fn tari_free(ptr: *mut u8) {
//     let mut len = [0u8; 4];
//     copy(ptr, len.as_mut_ptr(), 4);
//
//     let cap = (u32::from_le_bytes(len) + 4) as usize;
//     drop(Vec::<u8>::from_raw_parts(ptr, cap, cap));
// }
