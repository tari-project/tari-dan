//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

pub mod address_list;
pub mod bucket;
pub mod commit_result;
// pub mod confidential_bucket;
pub mod execution_result;
pub mod hashing;
pub mod instruction;
pub mod logs;
pub mod non_fungible;
pub mod resource;
pub mod resource_container;
pub mod substate;
pub mod vault;

mod template;

use tari_template_lib::Hash;
pub use template::{calculate_template_binary_hash, TemplateAddress};

pub const LAYER_TWO_TARI_RESOURCE_ADDRESS: Hash = Hash::from_array([1u8; 32]);
