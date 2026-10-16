//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::{BTreeSet, HashSet};

use ootle_network::Network;
use tari_engine_types::{
    SubstateVersion,
    commit_result::{FinalizeResult, TransactionResult},
    substate::{SubstateDiff, SubstateId, SubstateValue, hash_substate},
    transaction_receipt::{FinalizeOutcome, TransactionReceipt},
};
use tari_indexer_client::types::{ProvenTransactionReceipt, SubstateProof};
use tari_ootle_common_types::{Epoch, ShardGroup};
use tari_ootle_storage::{
    DownProofAnchor,
    TrustedStateRoot,
    consensus_models::{CommittedBlockProof, VerifiedBlockTip},
    decode_substate_down_proof,
    verify_substate_down_proof_against_roots,
    verify_substate_value_proof_against_root,
};
use tari_ootle_transaction::TransactionId;

use crate::{LightClientError, VerifiedCommittee};

impl VerifiedCommittee {
    /// Verifies that `substate_id` at `version` held `value` (or, for `None`, was absent) when the block `proof` is
    /// anchored at committed. This committee must be the anchor block's: of its epoch and shard group.
    ///
    /// Returns the anchor block's quorum-signed header fields. The proof says nothing about later blocks, so the caller
    /// judges freshness by the returned epoch and height.
    pub fn verify_substate(
        &self,
        substate_id: &SubstateId,
        version: SubstateVersion,
        value: Option<&SubstateValue>,
        proof: &SubstateProof,
    ) -> Result<VerifiedBlockTip, LightClientError> {
        let tip = self.verify_commit_proof(&proof.commit_proof)?;
        verify_substate_value_proof_against_root(
            &proof.value_proof,
            substate_id,
            version,
            value,
            self.network(),
            self.num_preshards(),
            proof.value_hash_epoch,
            &TrustedStateRoot::from(tip),
        )?;
        Ok(tip)
    }

    /// Verifies that `transaction_id` committed with the receipt `proven` carries. A transaction commits only once, so
    /// a proven receipt stays proven however old its anchor block is, given a committee of that block's epoch.
    pub fn verify_receipt(
        &self,
        transaction_id: TransactionId,
        proven: &ProvenTransactionReceipt,
    ) -> Result<VerifiedBlockTip, LightClientError> {
        let proof = proven
            .proof
            .as_ref()
            .ok_or_else(|| LightClientError::MissingReceiptProof {
                transaction_id: transaction_id.to_string(),
            })?;
        let substate_id = SubstateId::TransactionReceipt(transaction_id.into_receipt_address());
        let value = SubstateValue::from(proven.receipt.clone());
        self.verify_substate(&substate_id, proven.version, Some(&value), proof)
    }
}

/// The epoch and shard group of the block a down proof's inclusion half is anchored at, which select the committee to
/// verify it with. Untrusted until [`verify_down_proof`] succeeds with that committee.
pub fn down_proof_up_anchor(down_proof: &[u8]) -> Result<(Epoch, ShardGroup), LightClientError> {
    let decoded = decode_substate_down_proof(down_proof)?;
    let up_commit_proof = CommittedBlockProof::from_bytes(&decoded.up_commit_proof)?;
    Ok((up_commit_proof.epoch(), up_commit_proof.shard_group()?))
}

/// Verifies that `substate_id` at `version` was committed and has gone down by the block `down_commit_proof` proves.
///
/// `up_committee` must be the committee of the block the proof's inclusion half is anchored at (see
/// [`down_proof_up_anchor`]), and `down_committee` that of the block `down_commit_proof` proves.
///
/// The two anchors are usually in different epochs, each verified with a committee of its own epoch's validator set.
///
/// An honest node can serve a down proof that proves nothing: a global substate's proof with roots of two shard
/// groups. Such a proof fails with the [`SubstateProofVerifyError`](tari_ootle_storage::SubstateProofVerifyError)
/// that names the case, which leaves the down unproven without showing the indexer dishonest.
pub fn verify_down_proof(
    substate_id: &SubstateId,
    version: SubstateVersion,
    down_proof: &[u8],
    up_committee: &VerifiedCommittee,
    down_committee: &VerifiedCommittee,
    down_commit_proof: &[u8],
) -> Result<(), LightClientError> {
    let decoded = decode_substate_down_proof(down_proof)?;
    let up_tip = up_committee.verify_commit_proof(&decoded.up_commit_proof)?;
    let down_tip = down_committee.verify_commit_proof(down_commit_proof)?;
    verify_substate_down_proof_against_roots(
        down_proof,
        substate_id,
        version,
        down_committee.network(),
        down_committee.num_preshards(),
        &DownProofAnchor::from(up_tip),
        &DownProofAnchor::from(down_tip),
    )?;
    Ok(())
}

/// Checks that `result` is the result of the transaction whose verified receipt is `receipt`: it is `transaction_id`'s,
/// commits what the receipt's outcome says, with the receipt's events and fees, and its diff matches the receipt as
/// [`check_diff_matches_receipt`] checks.
///
/// The receipt does not commit to the result's `logs` or `total_fees_required`, so those stay as the indexer served
/// them.
pub fn check_result_matches_receipt(
    network: Network,
    transaction_id: TransactionId,
    receipt: &TransactionReceipt,
    result: &FinalizeResult,
) -> Result<(), LightClientError> {
    let mismatch = |details: String| LightClientError::ResultDoesNotMatchReceipt { details };
    if result.transaction_hash.into_array() != transaction_id.into_array() {
        return Err(mismatch(format!(
            "the result is of transaction {}, not {transaction_id}",
            result.transaction_hash
        )));
    }
    let diff = match (&result.result, receipt.outcome()) {
        (TransactionResult::Accept(diff), FinalizeOutcome::Commit) |
        (TransactionResult::AcceptFeeRejectRest(diff, _), FinalizeOutcome::FeeIntentCommit) => diff,
        (TransactionResult::Reject(_), _) => {
            return Err(mismatch("the result rejects a transaction that committed".to_string()));
        },
        (_, outcome) => {
            return Err(mismatch(format!(
                "the result commits a different part of the transaction than the receipt's {outcome:?}"
            )));
        },
    };
    if result.events.as_slice() != receipt.events() {
        return Err(mismatch("the result's events are not the receipt's".to_string()));
    }
    if result.fee_receipt != *receipt.fee_receipt() {
        return Err(mismatch("the result's fee receipt is not the receipt's".to_string()));
    }
    check_diff_matches_receipt(network, transaction_id, receipt, diff)
}

/// Checks that `diff` is the diff of the transaction whose verified receipt is `receipt`: it ups exactly the substate
/// versions and values the receipt commits to, plus the receipt itself, and downs exactly the substates the receipt
/// lists as downed.
pub fn check_diff_matches_receipt(
    network: Network,
    transaction_id: TransactionId,
    receipt: &TransactionReceipt,
    diff: &SubstateDiff,
) -> Result<(), LightClientError> {
    let receipt_id = SubstateId::TransactionReceipt(transaction_id.into_receipt_address());
    let epoch = receipt.epoch();

    let diff_ups = diff
        .up_iter()
        .filter(|(id, _)| *id != receipt_id)
        .map(|(id, substate)| {
            let value_hash = hash_substate(network, substate.substate_value(), substate.version(), epoch);
            (id, substate.version(), value_hash)
        })
        .collect::<BTreeSet<_>>();
    let receipt_ups = receipt
        .diff_summary()
        .upped
        .iter()
        .map(|up| (&up.substate_id, up.version, up.value_hash))
        .collect::<BTreeSet<_>>();
    check_same("up", &diff_ups, &receipt_ups, |(id, version, _)| {
        format!("{id}v{version}")
    })?;

    // The receipt leaves out the downs that an up of the same substate implies.
    let upped_ids = diff.up_iter().map(|(id, _)| id).collect::<HashSet<_>>();
    let diff_downs = diff
        .down_iter()
        .filter(|(id, _)| !upped_ids.contains(id))
        .map(|(id, version)| (id, *version))
        .collect::<BTreeSet<_>>();
    let receipt_downs = receipt
        .diff_summary()
        .downed
        .iter()
        .map(|down| (&down.substate_id, down.version))
        .collect::<BTreeSet<_>>();
    check_same("down", &diff_downs, &receipt_downs, |(id, version)| {
        format!("{id}v{version}")
    })
}

fn check_same<T: Ord>(
    change: &str,
    diff: &BTreeSet<T>,
    receipt: &BTreeSet<T>,
    describe: impl Fn(&T) -> String,
) -> Result<(), LightClientError> {
    if let Some(extra) = diff.difference(receipt).next() {
        return Err(LightClientError::DiffDoesNotMatchReceipt {
            details: format!(
                "the diff {change}s {}, which the receipt does not commit to",
                describe(extra)
            ),
        });
    }
    if let Some(missing) = receipt.difference(diff).next() {
        return Err(LightClientError::DiffDoesNotMatchReceipt {
            details: format!(
                "the receipt commits to a {change} of {}, which the diff does not carry",
                describe(missing)
            ),
        });
    }
    Ok(())
}
