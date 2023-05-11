//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
mod error;
pub mod kdfs;
mod proof;

pub use error::ConfidentialProofError;
pub use proof::{
    decrypt_value,
    generate_confidential_proof,
    get_commitment_factory,
    get_range_proof_service,
    ConfidentialProofStatement,
};
