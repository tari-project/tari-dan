// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

mod bootstrap;
pub mod fees;
pub mod flow;
pub mod function_definitions;
pub mod packager;
pub mod runtime;
pub mod state_store;
pub mod traits;
pub mod transaction;
pub mod wasm;

pub use bootstrap::bootstrap_state;
pub use tari_template_abi as abi;

pub mod base_layer_hashers {
    use tari_crypto::{hash::blake2::Blake256, hasher};
    // TODO: DRY - This should always be the same as the base layer hasher
    hasher!(
        Blake256,
        ConfidentialOutputHasher,
        "com.tari.layer_two.confidential_output",
        1,
        confidential_output_hasher
    );
}
