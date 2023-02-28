// Copyright 2022 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

// pub mod crypto;
// pub mod flow;
pub mod function_definitions;
pub mod packager;
pub mod runtime;
// pub mod state;
mod bootstrap;
pub mod state_store;
pub mod traits;
pub mod transaction;
pub mod wasm;

pub use bootstrap::bootstrap_state;
pub use tari_template_abi as abi;

pub mod base_layer_hashers {
    use tari_crypto::{hash::blake2::Blake256, hash_domain, hashing::DomainSeparatedHasher};
    hash_domain!(BurntOutputDomain, "burnt_output", 1);
    pub type BurntOutputDomainHasher = DomainSeparatedHasher<Blake256, BurntOutputDomain>;
}
