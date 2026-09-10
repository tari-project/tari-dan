//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use tari_consensus::messages::HotstuffMessage;
use tari_consensus_types::{BlockId, Decision};
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_storage::{StateStore, StorageError, consensus_models::Block};
use tari_ootle_transaction::TransactionId;

use crate::support::{MessageFilter, Test, TestAddress, Validator, logging::setup_logger};

/// The committee whose chain the assertions read. Only its proposals may be withheld: a block orphaned in
/// the other committee says nothing about this one's dummy fill.
const COMMITTEE: [&str; 3] = ["1", "2", "3"];
/// The validator whose store is inspected. It must never be starved of a proposal, and two of the three
/// votes it takes part in are one short of quorum.
const OBSERVER: &str = "1";

#[derive(Default)]
struct OrphanPlan {
    transaction_id: Option<TransactionId>,
    orphan_block_id: Option<BlockId>,
    orphan_height: Option<NodeHeight>,
    /// The orphan's command for the transaction, captured off the wire: a block that never gathers a QC is
    /// pruned from every store before the assertions run.
    orphan_command: Option<String>,
}

/// Denies one committee member the first proposal to carry a command for the transaction, and records what
/// that block said.
fn withhold_first_command_for(plan: Arc<Mutex<OrphanPlan>>) -> MessageFilter {
    let observer = TestAddress::new(OBSERVER);
    let committee: Vec<TestAddress> = COMMITTEE.into_iter().map(TestAddress::new).collect();

    Box::new(move |from, to, msg| {
        let HotstuffMessage::Proposal(proposal) = msg else {
            return true;
        };
        // The proposer already holds its own block, so starving it changes nothing.
        if to == from || *to == observer || !committee.contains(to) {
            return true;
        }
        let mut plan = plan.lock().unwrap();
        if plan.orphan_block_id.is_some() {
            return true;
        }
        let Some(transaction_id) = plan.transaction_id else {
            return true;
        };
        let Some(command) = proposal
            .block
            .commands()
            .iter()
            .find(|cmd| cmd.transaction().is_some_and(|atom| atom.id == transaction_id))
        else {
            return true;
        };
        log::info!("🔇 Withholding {} from {to}", proposal.block);
        plan.orphan_command = Some(command.to_string());
        plan.orphan_height = Some(proposal.block.height());
        plan.orphan_block_id = Some(*proposal.block.id());
        false
    })
}

/// The blocks from the leaf back to the oldest ancestor still held, oldest first. The height index holds one
/// entry per height, so this is the only view that excludes an orphan; blocks below the committed chain may
/// already have been pruned, so the walk ends at whatever the store holds.
fn canonical_chain(vn: &Validator) -> Vec<Block> {
    let leaf = vn.get_leaf_block();
    vn.state_store()
        .with_read_tx(|tx| {
            let mut chain = Vec::new();
            let mut cursor = *leaf.block_id();
            while let Some(block) = Block::get(tx, &cursor).optional()? {
                let parent = *block.parent();
                let is_genesis = block.height().is_zero();
                chain.push(block);
                if is_genesis {
                    break;
                }
                cursor = parent;
            }
            chain.reverse();
            Ok::<_, StorageError>(chain)
        })
        .unwrap()
}

fn describe(chain: &[Block]) -> String {
    chain
        .iter()
        .map(|b| {
            format!(
                "h={} dummy={} cmds={} id={}",
                b.height(),
                b.is_dummy(),
                b.commands().len(),
                &b.id().to_string()[..8]
            )
        })
        .collect::<Vec<_>>()
        .join("\n  ")
}

/// A leader filling a timeout gap with a dummy chain extends from the justify block, so every read for that
/// proposal must be taken there. The highest seen block is an orphan the candidate abandons, and commands
/// derived from it describe pool state no replica reproduces.
///
/// One committee member is denied the proposal that first carries a command for the transaction. That block
/// is stored and evaluated by the rest of the committee — so it is the highest seen block — but two of three
/// votes cannot reach quorum, so it gathers no QC and the pacemaker fills its height with a dummy. The
/// proposal built on that dummy must repeat the orphan's command: both are entitled to see only the justify
/// block's state.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dummy_fill_proposes_from_the_justify_block_not_the_orphan() {
    setup_logger();

    let plan = Arc::new(Mutex::new(OrphanPlan::default()));

    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|config_mut| {
            config_mut.missed_proposal_suspend_threshold = 10;
            config_mut.pacemaker_block_time = Duration::from_secs(2);
        })
        .add_committee(0, COMMITTEE.to_vec())
        .add_committee(1, vec!["4", "5", "6"])
        .with_message_filter(withhold_first_command_for(plan.clone()))
        .start()
        .await;

    let (tx, _, _) = test.send_transaction_to_all(Decision::Commit, 1, 2, 1).await;
    let tx_id = *tx.id();
    plan.lock().unwrap().transaction_id = Some(tx_id);

    test.start_epoch(Epoch(1)).await;

    loop {
        let (_, _, _, committed_height) = test.on_block_committed().await;
        if test.is_transaction_pool_empty() {
            break;
        }
        if committed_height > NodeHeight(30) {
            panic!("Transaction not finalized after {committed_height} blocks");
        }
    }

    test.stop();

    let (orphan_id, orphan_height, orphan_command) = {
        let plan = plan.lock().unwrap();
        (
            plan.orphan_block_id
                .expect("no proposal carrying the transaction was ever withheld"),
            plan.orphan_height.unwrap(),
            plan.orphan_command.clone().unwrap(),
        )
    };

    let chain = canonical_chain(test.get_validator(&TestAddress::new(OBSERVER)));
    let shape = describe(&chain);
    log::info!(
        "canonical chain:\n  {shape}\norphan: {} at {orphan_height}",
        &orphan_id.to_string()[..8]
    );

    assert!(
        !chain.iter().any(|b| *b.id() == orphan_id),
        "the withheld block was committed rather than orphaned; chain shape:\n  {shape}"
    );

    let by_id = chain.iter().map(|b| (*b.id(), b)).collect::<HashMap<_, _>>();
    let post_dummy = chain
        .iter()
        .find(|b| {
            !b.is_dummy() && b.height() > orphan_height && by_id.get(b.parent()).is_some_and(|parent| parent.is_dummy())
        })
        .unwrap_or_else(|| panic!("no proposal was built on a dummy chain above the orphan; chain shape:\n  {shape}"));

    let post_dummy_command = post_dummy
        .commands()
        .iter()
        .find(|cmd| cmd.transaction().is_some_and(|atom| atom.id == tx_id))
        .map(|cmd| cmd.to_string());

    assert_eq!(
        post_dummy_command.as_deref(),
        Some(orphan_command.as_str()),
        "proposal on the dummy chain must repeat the orphan's command. Orphan at height {orphan_height}, proposal at \
         height {}; chain shape:\n  {shape}",
        post_dummy.height(),
    );

    test.assert_clean_shutdown().await;
}
