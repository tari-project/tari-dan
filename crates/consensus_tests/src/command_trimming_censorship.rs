//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! BUGHUNT PoC: a `CommandsCommitProof` carries its commands as `Vec<CommandOrHash>` — each entry is
//! either a full `Command` or a bare hash of one. The merkle root authenticates each entry's *hash*
//! only (`CommandOrHash::hash` returns the same value for `Command(c)` and `Hash(c.hash())`), so a
//! single Byzantine relayer of an otherwise genuine, already-signed commit proof can swap a full
//! command for its bare hash without changing the root: every signature and QC still verify.
//!
//! The receiving shard group's iterators over "applicable" commands (`applicable_commands_iter`,
//! `full_commands_iter`) silently skip `Hash` entries. This test checks what actually happens to a
//! transaction whose relevant command is trimmed this way: does the receiving committee eventually
//! recover (timeout/abort) or does it genuinely hang.
//!
//! Setup: committee 0 (shard group A) holds input S; committee 1 (shard group B) holds input T and
//! owns output O. The transaction writes both inputs and produces O in B, so B must receive a command
//! involving S as a foreign command from A. A Byzantine transport rewrites that command's
//! `CommandOrHash::Command(cmd)` entry into `CommandOrHash::Hash(cmd.hash(protocol_version))` in every
//! foreign-proposal message (embedded or standalone) that carries it, leaving the merkle root intact.

use std::sync::{Arc, Mutex};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::Decision;
use tari_ootle_common_types::{Epoch, NodeHeight, SubstateLockType};
use tari_ootle_storage::consensus_models::CommandOrHash;
use tari_ootle_transaction::TransactionId;

use crate::support::{ExecuteSpec, Test, TestAddress, TestVnDestination, build_transaction, logging::setup_logger};

/// Trims every `Command` entry for `target_tx` into its bare hash, in every `ForeignProposal` the
/// message carries (embedded in a `Proposal`, or standalone). The merkle root is unaffected because
/// `CommandOrHash::hash` is defined to agree for both variants.
fn trim_command_for_tx(msg: &mut HotstuffMessage, target_tx: &TransactionId, trimmed: &Arc<Mutex<usize>>) {
    let apply = |fp: &mut tari_ootle_storage::consensus_models::ForeignProposal| {
        let Ok(protocol_version) = fp.protocol_version() else {
            return;
        };
        let commands = fp.commit_proof_mut().commands_mut();
        for entry in commands.iter_mut() {
            if let CommandOrHash::Command(cmd) = entry &&
                cmd.transaction().is_some_and(|t| t.id() == target_tx)
            {
                let hash = cmd.hash(protocol_version);
                *entry = CommandOrHash::Hash(hash);
                *trimmed.lock().unwrap() += 1;
            }
        }
    };
    match msg {
        HotstuffMessage::Proposal(m) => {
            for fp in &mut m.foreign_proposals {
                apply(fp);
            }
        },
        HotstuffMessage::ForeignProposal(m) => {
            apply(&mut m.proposal);
        },
        _ => {},
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn trimmed_foreign_command_outcome() {
    setup_logger();

    // Shared slot the Byzantine transport reads at message time; populated once the tx id is known.
    let target: Arc<Mutex<Option<TransactionId>>> = Arc::new(Mutex::new(None));
    let target_for_closure = target.clone();
    let trimmed_count = Arc::new(Mutex::new(0usize));
    let trimmed_for_closure = trimmed_count.clone();

    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2"])
        .with_message_mutator(Box::new(move |_from, _to, mut msg| {
            if let Some(tx_id) = target_for_closure.lock().unwrap().as_ref() {
                trim_command_for_tx(&mut msg, tx_id, &trimmed_for_closure);
            }
            msg
        }))
        .start()
        .await;

    // S lives in committee 0, T in committee 1. Both are honest, committed at version 0.
    let s = test
        .create_substates_on_vns(TestVnDestination::Committee(0), 1)
        .remove(0);
    let t = test
        .create_substates_on_vns(TestVnDestination::Committee(1), 1)
        .remove(0);
    let output_id = test.build_outputs_for_committee(1, 1).remove(0);

    let s_id = s.substate_id().clone();
    let t_id = t.substate_id().clone();

    let tx = build_transaction(vec![s.clone().into(), t.clone().into()]);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: tx.transaction().clone(),
        decision: Decision::Commit,
        fee: 100,
        input_locks: vec![
            (s_id.clone(), SubstateLockType::Write),
            (t_id.clone(), SubstateLockType::Write),
        ],
        new_outputs: vec![output_id.clone()],
        validator_fee_withdrawals: vec![],
    });

    // Arm the Byzantine transport now that the tx id is known.
    *target.lock().unwrap() = Some(*tx.id());

    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;

    test.start_epoch(Epoch(1)).await;

    // Run for a bounded number of blocks. Unlike the pledge-forgery PoC, we do NOT assume the
    // transaction commits — we are trying to find out whether it hangs, aborts, or eventually commits
    // via some recovery path.
    let mut resolved = false;
    for _ in 0..60u32 {
        test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            resolved = true;
            break;
        }
        let leaf1 = test.get_validator(&TestAddress::new("1")).get_leaf_block();
        let leaf2 = test.get_validator(&TestAddress::new("2")).get_leaf_block();
        if leaf1.height > NodeHeight(60) || leaf2.height > NodeHeight(60) {
            break;
        }
    }

    let num_trimmed = *trimmed_count.lock().unwrap();
    eprintln!(
        "PoC: trimmed {} command entry/entries for tx {} across the run. Transaction pool empty (resolved) = {}",
        num_trimmed,
        tx.id(),
        resolved
    );

    if resolved {
        test.assert_all_validators_have_decision(tx.id(), Decision::Commit)
            .await;
        eprintln!("RESULT: transaction resolved (COMMIT) despite the trimmed command — no permanent hang observed.");
    } else {
        eprintln!(
            "RESULT: transaction did NOT resolve after 60 blocks / {} trimmed entries — consistent with a permanent \
             hang (no recovery path for a trimmed foreign command).",
            num_trimmed
        );
    }

    test.stop();

    // The test intentionally does not assert a specific outcome: the whole point of running it is to
    // find out, honestly, whether this is a liveness bug (hang) or merely a delay/no-op (recovers).
    assert!(
        num_trimmed > 0,
        "the mutator never saw the target command — test setup did not exercise the bug"
    );
}
