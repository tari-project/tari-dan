//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
mod error;
pub mod kdfs;
mod proof;

pub use error::ConfidentialProofError;
pub(crate) use proof::{decrypt_value, encrypt_value, generate_confidential_proof};
pub use proof::{get_commitment_factory, ConfidentialProofStatement};
