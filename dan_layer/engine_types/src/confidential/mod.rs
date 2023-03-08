//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod claim;
mod proof;
mod validation;
mod withdraw;

pub use claim::ConfidentialClaim;
pub use proof::{challenges, get_commitment_factory, get_range_proof_service};
pub use validation::validate_confidential_proof;
pub use withdraw::{validate_confidential_withdraw, ConfidentialOutput, ValidatedConfidentialWithdrawProof};
