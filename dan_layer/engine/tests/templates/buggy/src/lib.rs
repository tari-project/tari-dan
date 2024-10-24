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
#![allow(non_snake_case)]

use core::ptr;

pub use tari_template_abi::tari_alloc;

#[global_allocator]
static ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

#[cfg(feature = "return_null_abi")]
#[no_mangle]
pub static _ABI_TEMPLATE_DEF: [u8; 0] = [];

#[cfg(feature = "return_empty_abi")]
#[no_mangle]
pub static _ABI_TEMPLATE_DEF: [u8; 4] = [0, 0, 0, 0];

#[cfg(not(any(feature = "return_empty_abi", feature = "return_null_abi")))]
#[no_mangle]
pub static _ABI_TEMPLATE_DEF: [u8; 446] = [
    186, 1, 0, 0, 161, 98, 86, 49, 163, 109, 116, 101, 109, 112, 108, 97, 116, 101, 95, 110, 97, 109, 101, 101, 83,
    116, 97, 116, 101, 108, 116, 97, 114, 105, 95, 118, 101, 114, 115, 105, 111, 110, 101, 48, 46, 55, 46, 48, 105,
    102, 117, 110, 99, 116, 105, 111, 110, 115, 133, 164, 100, 110, 97, 109, 101, 99, 110, 101, 119, 105, 97, 114, 103,
    117, 109, 101, 110, 116, 115, 128, 102, 111, 117, 116, 112, 117, 116, 161, 101, 79, 116, 104, 101, 114, 161, 100,
    110, 97, 109, 101, 105, 67, 111, 109, 112, 111, 110, 101, 110, 116, 102, 105, 115, 95, 109, 117, 116, 244, 164,
    100, 110, 97, 109, 101, 111, 99, 114, 101, 97, 116, 101, 95, 109, 117, 108, 116, 105, 112, 108, 101, 105, 97, 114,
    103, 117, 109, 101, 110, 116, 115, 129, 162, 100, 110, 97, 109, 101, 97, 110, 104, 97, 114, 103, 95, 116, 121, 112,
    101, 99, 85, 51, 50, 102, 111, 117, 116, 112, 117, 116, 100, 85, 110, 105, 116, 102, 105, 115, 95, 109, 117, 116,
    244, 164, 100, 110, 97, 109, 101, 106, 114, 101, 115, 116, 114, 105, 99, 116, 101, 100, 105, 97, 114, 103, 117,
    109, 101, 110, 116, 115, 128, 102, 111, 117, 116, 112, 117, 116, 161, 101, 79, 116, 104, 101, 114, 161, 100, 110,
    97, 109, 101, 105, 67, 111, 109, 112, 111, 110, 101, 110, 116, 102, 105, 115, 95, 109, 117, 116, 244, 164, 100,
    110, 97, 109, 101, 99, 115, 101, 116, 105, 97, 114, 103, 117, 109, 101, 110, 116, 115, 130, 162, 100, 110, 97, 109,
    101, 100, 115, 101, 108, 102, 104, 97, 114, 103, 95, 116, 121, 112, 101, 161, 101, 79, 116, 104, 101, 114, 161,
    100, 110, 97, 109, 101, 105, 38, 109, 117, 116, 32, 115, 101, 108, 102, 162, 100, 110, 97, 109, 101, 101, 118, 97,
    108, 117, 101, 104, 97, 114, 103, 95, 116, 121, 112, 101, 99, 85, 51, 50, 102, 111, 117, 116, 112, 117, 116, 100,
    85, 110, 105, 116, 102, 105, 115, 95, 109, 117, 116, 245, 164, 100, 110, 97, 109, 101, 99, 103, 101, 116, 105, 97,
    114, 103, 117, 109, 101, 110, 116, 115, 129, 162, 100, 110, 97, 109, 101, 100, 115, 101, 108, 102, 104, 97, 114,
    103, 95, 116, 121, 112, 101, 161, 101, 79, 116, 104, 101, 114, 161, 100, 110, 97, 109, 101, 101, 38, 115, 101, 108,
    102, 102, 111, 117, 116, 112, 117, 116, 99, 85, 51, 50, 102, 105, 115, 95, 109, 117, 116, 244,
];

#[no_mangle]
pub extern "C" fn Buggy_main(_call_info: *mut u8, _call_info_len: usize) -> *mut u8 {
    ptr::null_mut()
}

extern "C" {
    pub fn tari_engine(op: i32, input_ptr: *const u8, input_len: usize) -> *mut u8;
    pub fn debug(input_ptr: *const u8, input_len: usize);
    pub fn on_panic(msg_ptr: *const u8, msg_len: u32, line: u32, column: u32);
}

#[cfg(feature = "unexpected_export_function")]
#[no_mangle]
pub extern "C" fn i_shouldnt_be_here() -> *mut u8 {
    ptr::null_mut()
}
