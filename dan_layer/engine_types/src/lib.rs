//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

pub mod commit_result;
pub mod execution_result;
pub mod hashing;
pub mod instruction;
pub mod logs;
pub mod resource;
pub mod signature;
pub mod substate;

mod template;
pub use template::{calculate_template_binary_hash, TemplateAddress};
