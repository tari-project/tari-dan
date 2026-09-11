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

// The `engine_call_in_alloc_or_free` variants supply their own `tari_alloc`/`tari_free` pair, so
// they must not link `tari_template_abi`: that crate exports `tari_free` under the same
// `#[no_mangle]` symbol.
#[cfg(not(any(
    feature = "engine_call_in_alloc",
    feature = "engine_call_in_free",
    feature = "engine_call_in_response_alloc"
)))]
pub use tari_template_abi::tari_alloc;

#[global_allocator]
static ALLOC: lol_alloc::AssumeSingleThreaded<lol_alloc::FreeListAllocator> =
    unsafe { lol_alloc::AssumeSingleThreaded::new(lol_alloc::FreeListAllocator::new()) };

// Hard-coded minicbor encoding of `TemplateDef::V1(TemplateDefV1 { template_name: "Buggy",
// abi_version: 0, functions: [] })`. The leading 4 bytes are the little-endian length prefix used
// by `encode_for_wasm_embedding`. Regenerate by running the snippet preserved in
// `tests/test.rs::test_buggy_template` (currently commented out at the top of that function).
//
// The `#[template]` macro emits this section for a real template; it is written out by hand here so
// these variants can be malformed in ways the macro cannot express.
#[cfg(not(any(
    feature = "no_template_def",
    feature = "engine_call_in_alloc",
    feature = "engine_call_in_free",
    feature = "engine_call_in_response_alloc"
)))]
#[used]
#[unsafe(link_section = "tari_tdef")]
static _TARI_TEMPLATE_DEF: [u8; 16] = [
    16, 0, 0, 0, 130, 0, 129, 131, 101, 66, 117, 103, 103, 121, 0, 128,
];

#[cfg(not(any(
    feature = "engine_call_in_alloc",
    feature = "engine_call_in_free",
    feature = "engine_call_in_response_alloc"
)))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn Buggy_main(_call_info: *mut u8, _call_info_len: usize) -> *mut u8 {
    core::ptr::null_mut()
}

#[link(wasm_import_module = "env")]
unsafe extern "C" {
    pub fn tari_engine(op: i32, input_ptr: *const u8, input_len: usize) -> *mut u8;
    pub fn tari_debug(input_ptr: *const u8, input_len: usize);
    pub fn on_panic(msg_ptr: *const u8, msg_len: u32, line: u32, column: u32);
}

#[cfg(feature = "unexpected_export_function")]
#[unsafe(no_mangle)]
pub extern "C" fn i_shouldnt_be_here() -> *mut u8 {
    core::ptr::null_mut()
}

/// A template that re-enters the engine from `tari_alloc` or `tari_free`.
///
/// The engine drives both of those itself, at three points: `tari_alloc` to stage the `CallInfo`
/// before the invocation, `tari_alloc` again to write each engine call's response during it, and
/// `tari_free` on the pointer the template function returned after it. All three run template code
/// the engine called. An engine call from any of them must be refused, and refusing it must fail
/// the transaction even though this template ignores the null pointer it gets back.
///
/// The response allocation is the dangerous one: left open it cycles host -> WASM -> host once per
/// response and exhausts the native stack.
///
/// The memory layout mirrors `tari_template_abi`: an allocation is `[usize length prefix][payload]`
/// and the pointer handed to the engine points at the payload. It is reimplemented here rather
/// than reused so these variants do not link `tari_template_abi`, whose `tari_free` would collide
/// with the one below.
#[cfg(any(
    feature = "engine_call_in_alloc",
    feature = "engine_call_in_free",
    feature = "engine_call_in_response_alloc"
))]
mod engine_call_in_alloc_or_free {
    use std::alloc::{Layout, alloc, dealloc};

    use super::tari_engine;

    const USIZE_SIZE: usize = size_of::<usize>();
    const USIZE_ALIGN: usize = align_of::<usize>();

    /// Hard-coded minicbor encoding of `TemplateDef::V1(TemplateDefV1 { template_name: "Buggy",
    /// abi_version: 0, functions: [FunctionDef { name: "main", arguments: [], output: Type::Unit,
    /// is_mut: false, is_migration: false }] })`, with the 4-byte little-endian length prefix that
    /// `encode_for_wasm_embedding` adds. The one declared function makes the template callable, so
    /// the engine drives the `tari_alloc` of the `CallInfo` and the `tari_free` of the returned
    /// pointer.
    #[used]
    #[unsafe(link_section = "tari_tdef")]
    static _TARI_TEMPLATE_DEF: [u8; 28] = [
        28, 0, 0, 0, 130, 0, 129, 131, 101, 66, 117, 103, 103, 121, 0, 129, 133, 100, 109, 97, 105,
        110, 128, 130, 0, 128, 244, 244,
    ];

    /// Hard-coded minicbor encoding of `EmitLogArg { message: "call", level: LogLevel::Info }`.
    const EMIT_LOG_ARG: [u8; 9] = [130, 100, 99, 97, 108, 108, 130, 2, 128];

    /// `EngineOp::EmitLog`
    const OP_EMIT_LOG: i32 = 0x00;

    /// Minicbor encoding of `()`, the declared return type of `main`.
    const ENCODED_UNIT: [u8; 1] = [128];

    #[unsafe(no_mangle)]
    pub extern "C" fn tari_alloc(size: usize) -> *mut u8 {
        #[cfg(feature = "engine_call_in_alloc")]
        call_engine();
        // Skipping the `CallInfo` allocation, which is refused before the invocation begins, leaves
        // the response allocation as the one that runs while an invocation is in flight.
        #[cfg(feature = "engine_call_in_response_alloc")]
        if unsafe { core::ptr::read_volatile(&raw const IN_INVOCATION) } {
            call_engine();
        }
        internal_alloc(size)
    }

    #[cfg(feature = "engine_call_in_response_alloc")]
    static mut IN_INVOCATION: bool = false;

    /// # Safety
    /// `ptr` must point at the payload of an allocation made by [`tari_alloc`].
    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn tari_free(ptr: *mut u8) {
        #[cfg(feature = "engine_call_in_free")]
        call_engine();

        if !ptr.is_null() {
            unsafe { internal_free(ptr) };
        }
    }

    /// Calls the engine and discards whatever comes back, including a null pointer signalling that
    /// the engine refused the call. Any response is released directly rather than through
    /// `tari_free`, which would recurse without bound and say nothing about what the engine does
    /// with one re-entrant call.
    fn call_engine() {
        let response = unsafe { tari_engine(OP_EMIT_LOG, EMIT_LOG_ARG.as_ptr(), EMIT_LOG_ARG.len()) };
        if !response.is_null() {
            unsafe { internal_free(response) };
        }
    }

    #[unsafe(no_mangle)]
    pub unsafe extern "C" fn Buggy_main(_call_info: *mut u8, _call_info_len: usize) -> *mut u8 {
        #[cfg(feature = "engine_call_in_response_alloc")]
        {
            unsafe { core::ptr::write_volatile(&raw mut IN_INVOCATION, true) };
            // The response to this call is written through `tari_alloc` above, which calls the
            // engine again.
            call_engine();
            unsafe { core::ptr::write_volatile(&raw mut IN_INVOCATION, false) };
        }
        let ptr = internal_alloc(ENCODED_UNIT.len());
        unsafe { ptr.copy_from_nonoverlapping(ENCODED_UNIT.as_ptr(), ENCODED_UNIT.len()) };
        ptr
    }

    fn internal_alloc(size: usize) -> *mut u8 {
        let alloc_size = size + USIZE_SIZE;
        unsafe {
            let layout = Layout::from_size_align_unchecked(alloc_size, USIZE_ALIGN);
            let ptr = alloc(layout);
            ptr.cast::<usize>().write(alloc_size);
            ptr.add(USIZE_SIZE)
        }
    }

    unsafe fn internal_free(ptr: *mut u8) {
        unsafe {
            let alloc_ptr = ptr.sub(USIZE_SIZE);
            let alloc_size = alloc_ptr.cast::<usize>().read();
            dealloc(alloc_ptr, Layout::from_size_align_unchecked(alloc_size, USIZE_ALIGN));
        }
    }
}
