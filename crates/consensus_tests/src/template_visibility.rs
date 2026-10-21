//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests that a transaction calling a template resolves it from the chain it executes on, so that every validator
//! evaluating a block reaches the same decision for it.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::Duration,
};

use ootle_byte_type::ToByteType;
use tari_consensus::{hotstuff::HotstuffEvent, messages::HotstuffMessage};
use tari_consensus_types::Decision;
use tari_engine_types::{
    hashing::hash_template_code,
    published_template::PublishedTemplateAddress,
    substate::SubstateId,
};
use tari_ootle_common_types::{Epoch, NodeHeight, SubstateLockType, crypto::create_key_pair};
use tari_ootle_storage::consensus_models::TransactionRecord;
use tari_ootle_transaction::{Transaction, TransactionId};
use tokio::sync::watch;

use crate::support::{
    ExecuteSpec,
    MessageFilter,
    NetworkSendObserver,
    Test,
    TestAddress,
    TestVnDestination,
    load_binary_fixture,
    logging::setup_logger,
};

const COMMITTEE: [&str; 4] = ["1", "2", "3", "4"];

struct ProposalLog {
    proposers: Mutex<HashMap<NodeHeight, TestAddress>>,
    highest_proposed: watch::Sender<NodeHeight>,
    /// The transaction whose committing proposal is reported to `committing_height`.
    transaction_id: OnceLock<TransactionId>,
    committing_height: watch::Sender<Option<NodeHeight>>,
}

impl ProposalLog {
    fn new() -> Self {
        Self {
            proposers: Mutex::default(),
            highest_proposed: watch::Sender::new(NodeHeight::zero()),
            transaction_id: OnceLock::new(),
            committing_height: watch::Sender::new(None),
        }
    }

    fn proposer_at(&self, height: NodeHeight) -> Option<TestAddress> {
        self.proposers.lock().unwrap().get(&height).cloned()
    }

    async fn wait_for_proposal_at(&self, height: NodeHeight) {
        self.highest_proposed
            .subscribe()
            .wait_for(|h| *h >= height)
            .await
            .unwrap();
    }

    async fn wait_for_committing_height(&self) -> NodeHeight {
        self.committing_height
            .subscribe()
            .wait_for(Option::is_some)
            .await
            .unwrap()
            .unwrap()
    }
}

/// Records the proposer of each height, and reports the height of the first proposal that commits the logged
/// transaction.
fn log_proposals(log: Arc<ProposalLog>) -> NetworkSendObserver {
    Arc::new(move |from, _store, msg| {
        let HotstuffMessage::Proposal(proposal) = msg else {
            return;
        };
        let height = proposal.block.height();
        log.proposers
            .lock()
            .unwrap()
            .entry(height)
            .or_insert_with(|| from.clone());
        log.highest_proposed.send_if_modified(|h| {
            if height <= *h {
                return false;
            }
            *h = height;
            true
        });
        let Some(transaction_id) = log.transaction_id.get() else {
            return;
        };
        let commits_transaction = proposal
            .block
            .commands()
            .iter()
            .filter_map(|cmd| cmd.committing())
            .any(|atom| atom.id() == transaction_id);
        if commits_transaction {
            log.committing_height.send_if_modified(|h| {
                if h.is_some() {
                    return false;
                }
                *h = Some(height);
                true
            });
        }
    })
}

#[derive(Default)]
struct Withholding {
    height: Option<NodeHeight>,
    from: Vec<TestAddress>,
}

/// Withholds the proposal at the planned height from the planned validators.
fn withhold_proposal(plan: Arc<Mutex<Withholding>>) -> MessageFilter {
    Box::new(move |_from, to, msg| {
        let HotstuffMessage::Proposal(proposal) = msg else {
            return true;
        };
        let plan = plan.lock().unwrap();
        let withheld = plan.height == Some(proposal.block.height()) && plan.from.contains(to);
        if withheld {
            log::info!("🔇 Withholding {} from {to}", proposal.block);
        }
        !withheld
    })
}

struct TemplateCall {
    publish: TransactionRecord,
    call: TransactionRecord,
}

/// Builds a transaction publishing a template and one calling it, with executions that commit both while the
/// template resolves.
fn publish_and_call(test: &Test) -> TemplateCall {
    let (sk, pk) = create_key_pair();
    let wasm = load_binary_fixture("state.wasm");
    let template =
        PublishedTemplateAddress::from_author_and_binary_hash(&pk.to_byte_type(), &hash_template_code(&wasm));

    let publish_input = test.create_substates_on_vns(TestVnDestination::All, 1).remove(0);
    let publish = Transaction::builder_localnet(Epoch(1))
        .publish_template(wasm)
        .with_inputs([publish_input.clone().into()])
        .build_and_seal(&sk);
    let publish = TransactionRecord::new(publish);

    let call_input = test.create_substates_on_vns(TestVnDestination::All, 1).remove(0);
    let call = Transaction::builder_localnet(Epoch(1))
        .with_inputs([call_input.clone().into()])
        .build_and_seal(&create_key_pair().0);
    let call = TransactionRecord::new(call);

    test.add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: publish.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![(publish_input.into_substate_id(), SubstateLockType::Write)],
        new_outputs: vec![SubstateId::Template(template)],
        validator_fee_withdrawals: vec![],
        calls_template: None,
    })
    .add_execution_at_destination(TestVnDestination::All, ExecuteSpec {
        transaction: call.transaction().clone(),
        decision: Decision::Commit,
        fee: 1,
        input_locks: vec![(call_input.into_substate_id(), SubstateLockType::Write)],
        new_outputs: vec![],
        validator_fee_withdrawals: vec![],
        calls_template: Some(template),
    });

    TemplateCall { publish, call }
}

/// Starts the epoch and waits for a full leader rotation, so the leader of any later height can be read off the
/// proposer of the same height one rotation earlier.
async fn start_and_rotate_leaders(test: &mut Test, log: &ProposalLog) {
    test.start_epoch(Epoch(1)).await;
    while log.proposer_at(NodeHeight(COMMITTEE.len() as u64)).is_none() {
        test.on_block_committed().await;
    }
}

/// Runs until every validator has finalized `transaction_id`, failing on a consensus failure, and on a leader
/// timeout unless `timeouts_expected`.
async fn run_until_finalized(
    test: &mut Test,
    transaction_id: &TransactionId,
    max_height: NodeHeight,
    timeouts_expected: bool,
) {
    loop {
        let (address, event) = tokio::time::timeout(Duration::from_secs(60), test.on_hotstuff_event())
            .await
            .expect("Timed out waiting for a hotstuff event");
        match event {
            HotstuffEvent::LeaderTimeout { height } if !timeouts_expected => {
                panic!("[{address}] Leader timed out at {height}: the committee did not reach a QC")
            },
            HotstuffEvent::Failure { message } => panic!("[{address}] Consensus failure: {message}"),
            _ => {},
        }

        if test.is_transaction_finalized_at_destination(TestVnDestination::All, transaction_id) {
            return;
        }
        let leaf = test.get_validator(&TestAddress::new(COMMITTEE[0])).get_leaf_block();
        assert!(
            leaf.height <= max_height,
            "{transaction_id} was not finalized by {}",
            leaf.height
        );
    }
}

/// The validators that executed `transaction_id` without finding the template it calls, with how often they did.
fn template_misses(test: &Test, transaction_id: &TransactionId) -> Vec<(TestAddress, usize)> {
    let mut misses = test
        .validators_iter()
        .map(|vn| {
            (
                vn.address.clone(),
                vn.transaction_executor.get_template_miss_count(transaction_id),
            )
        })
        .filter(|(_, n)| *n > 0)
        .collect::<Vec<_>>();
    misses.sort();
    misses
}

/// A template published in block K is resolvable by a call in any block that descends from K, whether or not K has
/// committed yet. K commits three blocks later, so a call in K+3 executes before any validator has committed it.
///
/// The call is sent only to the leader of K+3. Leaders rotate through the four validators, so that is also the
/// leader of K-1: it proposes nothing between receiving the call and building K+3, and K+3 is the first block to
/// carry the call.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_call_resolves_a_template_published_in_an_uncommitted_ancestor() {
    setup_logger();

    let log = Arc::new(ProposalLog::new());
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|c| c.pacemaker_block_time = Duration::from_secs(2))
        .add_committee(0, COMMITTEE.to_vec())
        .with_send_observer(log_proposals(log.clone()))
        .start()
        .await;

    let TemplateCall { publish, call } = publish_and_call(&test);
    log.transaction_id.set(*publish.id()).unwrap();

    start_and_rotate_leaders(&mut test, &log).await;
    test.send_transaction_to_destination(TestVnDestination::All, publish.clone())
        .await;
    let publish_height = log.wait_for_committing_height().await;
    let call_height = publish_height + NodeHeight(3);
    let call_leader = log
        .proposer_at(publish_height - NodeHeight(1))
        .expect("the block before the publish was proposed");
    log::info!("Publish proposed at {publish_height}; sending the call to {call_leader}, the leader of {call_height}");
    test.send_transaction_to_destination(TestVnDestination::Address(call_leader.clone()), call.clone())
        .await;

    run_until_finalized(&mut test, call.id(), call_height + NodeHeight(10), false).await;

    test.stop();
    assert_eq!(
        log.proposer_at(call_height),
        Some(call_leader),
        "the leader of {call_height} is not the leader of the block before the publish"
    );
    assert_eq!(
        template_misses(&test, call.id()),
        vec![],
        "validators executed the call without the template it calls"
    );
    test.assert_all_validators_have_decision(publish.id(), Decision::Commit)
        .await;
    test.assert_all_validators_have_decision(call.id(), Decision::Commit)
        .await;
    test.assert_clean_shutdown().await;
}

/// Processing a proposal commits its 3-chain ancestor even when the proposal itself never gathers a QC. So a
/// validator that received an orphaned proposal has committed one block further than one that did not, and a
/// template lookup against committed state answers differently on the two.
///
/// The template is published in K. The proposal at K+3, whose processing commits K, is withheld from the leader of
/// K+5 and one other validator, so it gathers two of the three votes it needs and is orphaned. K+4 times out for
/// want of a QC on K+3, and the leader of K+5 fills both with dummies and proposes the call having committed only
/// K-1, while the two validators that processed the orphan have committed K. Every one of them must resolve the
/// template, because K is an ancestor of the block the call executes in.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn validators_that_processed_an_orphan_agree_with_those_that_did_not() {
    setup_logger();

    let log = Arc::new(ProposalLog::new());
    let withholding = Arc::new(Mutex::new(Withholding::default()));
    let mut test = Test::builder()
        .with_test_timeout(Duration::from_secs(60))
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(2);
            c.missed_proposal_suspend_threshold = 10;
        })
        .add_committee(0, COMMITTEE.to_vec())
        .with_send_observer(log_proposals(log.clone()))
        .with_message_filter(withhold_proposal(withholding.clone()))
        .start()
        .await;

    let TemplateCall { publish, call } = publish_and_call(&test);
    log.transaction_id.set(*publish.id()).unwrap();

    start_and_rotate_leaders(&mut test, &log).await;
    test.send_transaction_to_destination(TestVnDestination::All, publish.clone())
        .await;
    let publish_height = log.wait_for_committing_height().await;
    let orphan_height = publish_height + NodeHeight(3);
    let call_height = orphan_height + NodeHeight(2);
    // Leaders rotate through the four validators, so the leader of K+5 is the leader of K+1 and the leader of K+3
    // is the leader of K-1.
    log.wait_for_proposal_at(publish_height + NodeHeight(1)).await;
    let call_leader = log.proposer_at(publish_height + NodeHeight(1)).unwrap();
    let orphan_leader = log.proposer_at(publish_height - NodeHeight(1)).unwrap();
    let also_starved = COMMITTEE
        .iter()
        .map(|a| TestAddress::new(*a))
        .find(|a| *a != call_leader && *a != orphan_leader)
        .unwrap();
    log::info!(
        "Publish proposed at {publish_height}; withholding {orphan_height} from {call_leader} and {also_starved}"
    );
    *withholding.lock().unwrap() = Withholding {
        height: Some(orphan_height),
        from: vec![call_leader.clone(), also_starved],
    };

    log.wait_for_proposal_at(orphan_height).await;
    // The leader of K+1 proposed before the call reached it, and it proposes next at K+5.
    test.send_transaction_to_destination(TestVnDestination::Address(call_leader.clone()), call.clone())
        .await;

    run_until_finalized(&mut test, call.id(), call_height + NodeHeight(20), true).await;

    test.stop();
    assert_eq!(
        log.proposer_at(call_height),
        Some(call_leader),
        "the leader of {call_height} is not the leader of the block after the publish"
    );
    assert_eq!(
        template_misses(&test, call.id()),
        vec![],
        "validators executed the call without the template it calls"
    );
    test.assert_all_validators_have_decision(call.id(), Decision::Commit)
        .await;
    test.assert_clean_shutdown().await;
}
