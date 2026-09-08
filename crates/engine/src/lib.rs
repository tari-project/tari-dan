// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

pub mod abi_metrics;
pub mod executables;
pub mod fees;
pub mod intrinsics;
pub mod runtime;
pub mod state_store;
pub mod template;
pub mod traits;
pub mod transaction;
pub mod wasm;

pub use tari_engine_types as types;
pub use tari_template_abi as abi;

pub mod base_layer_hashers {
    use blake2::{Blake2b, digest::consts::U32};
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
