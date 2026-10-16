//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Verifies what a Tari Ootle indexer serves against the validator set handover chain, without trusting the indexer.
//!
//! Each end-of-epoch block names the next epoch's validator set, and its committee's quorum certificate commits to
//! that name. Starting from an anchor, a light client walks those handovers to the current epoch's
//! [`TrustedValidatorSet`], proves the committees it needs against it, and checks commit proofs, substate proofs and
//! receipts with those committees.

mod committee;
mod error;
mod proofs;
mod validator_set;
mod walk;

pub use committee::VerifiedCommittee;
pub use error::LightClientError;
pub use proofs::{check_diff_matches_receipt, down_proof_up_anchor, verify_down_proof};
pub use validator_set::TrustedValidatorSet;
pub use walk::{DEFAULT_MAX_WALK_EPOCHS, WalkPlan, plan_walk};
