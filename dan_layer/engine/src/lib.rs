// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

// FIXME: RuntimeError is at least 144 bytes
#![allow(clippy::result_large_err)]

mod bootstrap;
pub mod fees;
pub mod flow;
pub mod function_definitions;
pub mod runtime;
pub mod state_store;
pub mod template;
pub mod traits;
pub mod transaction;
pub mod wasm;

pub use bootstrap::bootstrap_state;
pub use tari_template_abi as abi;

pub mod base_layer_hashers {
    use blake2::{digest::consts::U32, Blake2b};
    use tari_crypto::hasher;
    // TODO: DRY - This should always be the same as the base layer hasher
    hasher!(
        Blake2b<U32>,
        ConfidentialOutputHasher,
        "com.tari.layer_two.confidential_output",
        1,
        confidential_output_hasher
    );
}
