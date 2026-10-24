//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
#[cfg(test)]
mod catch_up_no_vote;
#[cfg(test)]
mod catch_up_on_proposal_ahead;
#[cfg(test)]
mod catch_up_rewind;
#[cfg(test)]
mod consensus;
#[cfg(test)]
mod deferred_lock_conflict;
#[cfg(test)]
mod dummy_blocks;
#[cfg(test)]
mod dummy_fill_anchor;
#[cfg(test)]
mod epoch_change;
#[cfg(test)]
mod epoch_end_drain;
#[cfg(test)]
mod extends_justify;
#[cfg(test)]
mod foreign_prepare_timeout;
#[cfg(test)]
mod foreign_proposal_committee;
#[cfg(test)]
mod foreign_proposal_size;
#[cfg(test)]
mod high_pc_block_binding;
#[cfg(test)]
mod last_voted_persistence;
#[cfg(test)]
mod leader_failure;
#[cfg(test)]
mod leader_fast_skip;
#[cfg(test)]
mod parallel_execution;
#[cfg(test)]
mod peer_message_admission;
#[cfg(test)]
mod proposal_certificate_signatures;
#[cfg(test)]
mod safe_node_predicate;
#[cfg(test)]
mod stale_qc_carry_forward;
#[cfg(test)]
mod state_tree;
#[cfg(test)]
mod substate_store;
#[cfg(test)]
mod support;
#[cfg(test)]
mod template_visibility;
#[cfg(test)]
mod timeout_certificate;
#[cfg(test)]
mod timeout_certificate_signatures;
#[cfg(test)]
mod vote_equivocation_withholding;
