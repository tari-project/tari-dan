//  Copyright 2025 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use super::*;

    pub struct PushItToTheLimit {
        data: Bytes,
        items: Vec<u32>,
    }

    impl PushItToTheLimit {
        pub fn new(data: Bytes) -> Self {
            Self { data, items: Vec::new() }
        }

        pub fn set_data(&mut self, data: Bytes) {
            self.data = data;
        }

        /// Sets this component's items to `len` zeros, each a one-byte CBOR item in its state.
        pub fn set_items(&mut self, len: u32) {
            self.items = vec![0; len as usize];
        }

        /// Passes `len` zeros to `count_items` on `template` and returns the count that comes back.
        pub fn pass_items_nested(template: TemplateAddress, len: u32) -> u32 {
            TemplateManager::get(template).call("count_items", args![vec![0u32; len as usize]])
        }

        pub fn count_items(items: Vec<u32>) -> u32 {
            items.len() as u32
        }


        /// Returns `len` zeros, each encoded as a one-byte CBOR item.
        pub fn return_items(len: u32) -> Vec<u32> {
            vec![0; len as usize]
        }

        /// Calls `return_items` on `template` and returns how many items came back.
        pub fn return_items_nested(template: TemplateAddress, len: u32) -> u32 {
            let items: Vec<u32> = TemplateManager::get(template).call("return_items", args![len]);
            items.len() as u32
        }

        /// Calls the engine once with an argument of `items` one-byte CBOR items, which no engine op
        /// accepts, and ignores the refusal.
        pub fn engine_call_with_items(items: u32) {
            let mut arg = vec![0u8; 5 + items as usize];
            // The head of a definite-length array with a four-byte length.
            arg[0] = 0x9a;
            arg[1..5].copy_from_slice(&items.to_be_bytes());
            unsafe {
                tari_template_abi::tari_engine(
                    tari_template_abi::EngineOp::EmitLog.as_i32(),
                    arg.as_ptr(),
                    arg.len(),
                );
            }
        }

        pub fn request_random_bytes(len: u32) -> Vec<u8> {
            rand::random_bytes(len)
        }

        /// Hands the engine a log message of `len` bytes. This costs a template the least memory an
        /// engine call's argument can: the message and its encoding.
        pub fn engine_log_of_size(len: u32) {
            tari_template_lib::engine().emit_log(tari_template_lib::macros::LogLevel::Info, "a".repeat(len as usize));
        }

        pub fn emit_log_of_size(len: u32) {
            debug!("{}", "a".repeat(len as usize));
        }

        /// Emits an event whose payload holds a single `len`-byte value under the key `data`.
        pub fn emit_event_of_size(len: u32) {
            let mut payload = Metadata::new();
            payload.insert("data", &"a".repeat(len as usize));
            emit_event("big", payload);
        }
    }
}
