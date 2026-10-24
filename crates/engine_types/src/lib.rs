//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

pub mod bucket;
pub mod commit_result;
pub mod component;
pub mod confidential;
pub mod confidential_output;
pub mod crypto;
pub mod encoded;
pub mod events;
pub mod fees;
pub mod hashing;
pub mod indexed_value;
pub mod limits;
pub mod lock;
pub mod logs;
pub mod non_fungible;
pub mod ownership;
pub mod proof;
pub mod resource;
pub mod resource_container;
pub mod stealth;
pub mod substate;
pub mod transaction_receipt;
pub mod vault;
pub mod virtual_substate;

pub mod template;

/// Reading a template's ABI straight out of its WASM, without a compiler. Optional because it is
/// the one thing here that needs a WASM parser, and most consumers of this crate never read a
/// template binary.
#[cfg(feature = "extract-template-def")]
pub mod static_template_def;

pub mod entity_id_provider;
mod epoch;
pub mod id_provider;

mod borsh;
mod hash;
pub mod json_cbor;
mod protocol_version;
pub mod published_template;
mod substate_hasher;
mod substate_serde;
mod substate_version;
mod utxo;
mod validator_fee;

pub use epoch::*;
pub use hash::*;
pub use protocol_version::*;
pub use substate_version::*;
pub use tari_template_lib::types::parse_template_address;
pub use template::calculate_template_binary_hash;
pub use utxo::*;
pub use validator_fee::*;
