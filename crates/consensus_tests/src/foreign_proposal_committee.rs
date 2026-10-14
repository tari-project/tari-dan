//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Tests for which committee a foreign proposal's proof is verified against.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};

use ootle_byte_type::ToByteType;
use tari_common_types::types::{CompressedPublicKey, PrivateKey};
use tari_consensus::{
    hotstuff::{HotStuffError, ProposalValidationError},
    messages::{HotstuffMessage, ProposalMessage},
    resolve_foreign_committee,
};
use tari_consensus_types::{BlockId, Decision};
use tari_engine_types::substate::{Substate, SubstateValue};
use tari_ootle_common_types::{
    Epoch,
    ShardGroup,
    SubstateLockType,
    VotePower,
    committee::{Committee, CommitteeMember},
};
use tari_ootle_storage::{
    StateStore,
    consensus_models::{BlockPledge, CommandsCommitProof, ForeignProposal, ForeignProposalRecord},
};
use tari_ootle_transaction::{Network, Transaction};
use tari_sidechain::{SidechainBlockCommitProof, SidechainBlockHeader};
use tokio::{
    sync::broadcast,
    time::{sleep, timeout},
};

use crate::support::{
    TEST_NUM_PRESHARDS,
    Test,
    TestAddress,
    TestEpochManager,
    TestVnDestination,
    build_transaction_from,
    committee_number_to_shard_group,
    helpers,
    logging::setup_logger,
};

fn committee(addresses: &[&'static str]) -> Committee<TestAddress> {
    Committee::new(
        addresses
            .iter()
            .map(|addr| {
                let address = TestAddress::new(*addr);
                let (_, public_key) = helpers::derive_keypair_from_address(&address);
                CommitteeMember {
                    address,
                    public_key: public_key.to_byte_type(),
                    vote_power: VotePower::of(1),
                }
            })
            .collect(),
    )
}

/// A proposal whose header names `shard_group` and whose proposer is `proposed_by`.
fn proposal(proposed_by: &'static str, shard_group: tari_sidechain::ShardGroup) -> ForeignProposal {
    let (_, proposer) = helpers::derive_keypair_from_address(&TestAddress::new(proposed_by));
    let commit_proof = CommandsCommitProof::new_latest(vec![], SidechainBlockCommitProof {
        header: SidechainBlockHeader {
            network: Network::LocalNet.as_byte(),
            protocol_version: 0,
            parent_id: Default::default(),
            justify_id: Default::default(),
            height: 1,
            epoch: 1,
            epoch_hash: Default::default(),
            shard_group,
            proposed_by: CompressedPublicKey::new_from_pk(proposer),
            state_merkle_root: Default::default(),
            command_merkle_root: Default::default(),
            transaction_merkle_root: None,
            signature: Default::default(),
            accumulated_data: Default::default(),
            metadata_hash: Default::default(),
        },
        proof_elements: vec![],
    });
    ForeignProposal::new(commit_proof, BlockPledge::default())
}

fn to_header_shard_group(shard_group: ShardGroup) -> tari_sidechain::ShardGroup {
    tari_sidechain::ShardGroup {
        start: shard_group.start().as_u32(),
        end_inclusive: shard_group.end().as_u32(),
    }
}

struct Setup {
    epoch_manager: TestEpochManager,
    group_b: ShardGroup,
    committee_b: Committee<TestAddress>,
}

async fn setup() -> Setup {
    let group_a = committee_number_to_shard_group(TEST_NUM_PRESHARDS, 0, 2);
    let group_b = committee_number_to_shard_group(TEST_NUM_PRESHARDS, 1, 2);
    let committee_a = committee(&["a1", "a2", "a3"]);
    let committee_b = committee(&["b1", "b2", "b3"]);

    let (tx_events, _) = broadcast::channel(1);
    let epoch_manager = TestEpochManager::new(tx_events);
    epoch_manager
        .add_committees(HashMap::from([(group_a, committee_a), (group_b, committee_b.clone())]))
        .await;

    Setup {
        epoch_manager,
        group_b,
        committee_b,
    }
}

#[tokio::test]
async fn it_verifies_against_the_named_shard_groups_committee_not_the_proposers() {
    let Setup {
        epoch_manager,
        group_b,
        committee_b,
    } = setup().await;

    // A member of shard group A proposes a block whose header names shard group B
    let proposal = proposal("a1", to_header_shard_group(group_b));

    let committee = resolve_foreign_committee(&epoch_manager, &proposal)
        .await
        .unwrap()
        .expect("shard group B has a committee");
    assert_eq!(*committee, committee_b);
    assert!(!committee.contains_public_key(&proposal.proposed_by()));
}

#[tokio::test]
async fn it_finds_no_committee_for_a_shard_group_that_is_not_assigned() {
    let Setup {
        epoch_manager, group_b, ..
    } = setup().await;

    // A sub-range of shard group B has valid bounds but no committee of its own
    let sub_range = tari_sidechain::ShardGroup {
        start: group_b.start().as_u32(),
        end_inclusive: group_b.start().as_u32() + 1,
    };
    let proposal = proposal("b1", sub_range);

    let committee = resolve_foreign_committee(&epoch_manager, &proposal).await.unwrap();
    assert!(committee.is_none());
}

/// The same proposal with its commit proof stripped of the foreign committee's signatures.
fn without_commit_signatures(proposal: &ForeignProposal) -> ForeignProposal {
    let commit_proof = proposal.commit_proof();
    ForeignProposal::new(
        CommandsCommitProof::new_latest(commit_proof.commands().to_vec(), SidechainBlockCommitProof {
            header: commit_proof.sidechain_block_commit_proof().header.clone(),
            proof_elements: vec![],
        }),
        proposal.block_pledge().clone(),
    )
}

/// Starts committee 0 (`1`) with a transaction's input and committee 1 (`2`, `3`) with its output, and returns the
/// first local proposal in committee 1 that embeds a foreign proposal, withheld from replica `3` along with every
/// other route to that foreign proposal. `3` holds the transaction only if `replica_has_transaction`.
async fn withheld_embedding_proposal(
    replica_has_transaction: bool,
) -> (Test, TestAddress, TestAddress, ProposalMessage) {
    let replica = TestAddress::new("3");
    let captured = Arc::new(Mutex::new(None::<(TestAddress, ProposalMessage)>));
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2", "3"])
        .with_message_filter(Box::new({
            let replica = replica.clone();
            let captured = captured.clone();
            move |from, to, msg| {
                if *to != replica {
                    return true;
                }
                // The replica only learns of the foreign proposal through the local proposal the test delivers
                match msg {
                    HotstuffMessage::ForeignProposal(_) | HotstuffMessage::CatchUpSyncResponse(_) => false,
                    HotstuffMessage::Proposal(proposal) if !proposal.foreign_proposals.is_empty() => {
                        captured
                            .lock()
                            .unwrap()
                            .get_or_insert_with(|| (from.clone(), (**proposal).clone()));
                        false
                    },
                    _ => true,
                }
            }
        }))
        .start()
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 1);
    let outputs = test.build_outputs_for_committee(1, 1);
    let tx = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .build_and_seal(&PrivateKey::default()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx,
        Decision::Commit,
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs,
    );
    if replica_has_transaction {
        test.send_transaction_to_destination(TestVnDestination::All, tx).await;
    } else {
        for address in ["1", "2"] {
            test.send_transaction_to_destination(TestVnDestination::Address(TestAddress::new(address)), tx.clone())
                .await;
        }
    }
    test.start_epoch(Epoch(1)).await;

    let (leader, proposal) = timeout(Duration::from_secs(60), async {
        loop {
            if let Some(captured) = captured.lock().unwrap().take() {
                return captured;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("committee 1 never proposed a block carrying a foreign proposal");

    (test, replica, leader, proposal)
}

fn is_foreign_proposal_stored(test: &Test, address: &TestAddress, block_ids: &[BlockId]) -> bool {
    test.get_validator(address)
        .state_store()
        .with_read_tx(|tx| {
            block_ids
                .iter()
                .map(|id| ForeignProposalRecord::record_exists(tx, id))
                .collect::<Result<Vec<_>, _>>()
        })
        .unwrap()
        .into_iter()
        .any(|stored| stored)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn an_embedded_foreign_proposal_is_not_stored_without_its_committees_commit_proof() {
    setup_logger();
    let (mut test, replica, leader, proposal) = withheld_embedding_proposal(true).await;

    let foreign_proposals = proposal
        .foreign_proposals
        .iter()
        .map(without_commit_signatures)
        .collect::<Vec<_>>();
    let block_ids = foreign_proposals
        .iter()
        .map(|fp| fp.calculate_block_id())
        .collect::<Vec<_>>();
    let tampered = ProposalMessage {
        block: proposal.block,
        foreign_proposals,
    };
    let validator = test.get_validator(&replica);
    validator
        .tx_inbound_message
        .send((leader, HotstuffMessage::new_proposal(tampered)))
        .await
        .unwrap();
    sleep(Duration::from_secs(3)).await;

    let stored = is_foreign_proposal_stored(&test, &replica, &block_ids);
    test.stop();
    assert!(!stored, "replica stored an unauthenticated foreign proposal");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_block_with_an_unauthenticated_embedded_foreign_proposal_does_not_take_the_parked_slot() {
    setup_logger();
    // The replica lacks the transaction, so both copies of the block are parked while it fetches it
    let (mut test, replica, leader, proposal) = withheld_embedding_proposal(false).await;

    let block_ids = proposal
        .foreign_proposals
        .iter()
        .map(|fp| fp.calculate_block_id())
        .collect::<Vec<_>>();
    let tampered = ProposalMessage {
        block: proposal.block.clone(),
        foreign_proposals: proposal
            .foreign_proposals
            .iter()
            .map(without_commit_signatures)
            .collect(),
    };
    let tx_inbound = test.get_validator(&replica).tx_inbound_message.clone();
    tx_inbound
        .send((leader.clone(), HotstuffMessage::new_proposal(tampered)))
        .await
        .unwrap();
    tx_inbound
        .send((leader, HotstuffMessage::new_proposal(proposal)))
        .await
        .unwrap();

    let stored = timeout(Duration::from_secs(15), async {
        while !is_foreign_proposal_stored(&test, &replica, &block_ids) {
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .is_ok();
    test.stop();
    assert!(
        stored,
        "the genuine block was not processed after an unauthenticated copy"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_foreign_proposal_from_a_peer_that_was_not_asked_for_it_is_discarded() {
    setup_logger();
    let (mut test, replica, leader, proposal) = withheld_embedding_proposal(true).await;

    // The replica only ever asks members of the foreign committee, so a copy from a local member is unsolicited
    let foreign_proposal = proposal
        .foreign_proposals
        .first()
        .expect("the captured proposal embeds a foreign proposal")
        .clone();
    let block_id = foreign_proposal.calculate_block_id();
    test.get_validator(&replica)
        .tx_inbound_message
        .send((leader, HotstuffMessage::ForeignProposal(foreign_proposal.into())))
        .await
        .unwrap();
    sleep(Duration::from_secs(3)).await;

    let stored = is_foreign_proposal_stored(&test, &replica, &[block_id]);
    test.stop();
    assert!(!stored, "replica processed a foreign proposal it did not request");
}

/// The same proposal, with every pledged component's state replaced.
fn with_substituted_pledge_values(proposal: &ForeignProposal) -> ForeignProposal {
    let mut pledge = serde_json::to_value(proposal.block_pledge()).unwrap();
    let mut num_substituted = 0;
    for substate in pledge["pledges"].as_object_mut().unwrap().values_mut() {
        let mut value: Substate = serde_json::from_value(substate.clone()).unwrap();
        if let SubstateValue::Component(component) = value.substate_value_mut() {
            component.body.state = tari_bor::Value::Integer(1_000_000_000.into());
            *substate = serde_json::to_value(value).unwrap();
            num_substituted += 1;
        }
    }
    assert!(num_substituted > 0, "no component pledged");
    ForeignProposal::new(proposal.commit_proof().clone(), serde_json::from_value(pledge).unwrap())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_foreign_proposal_is_not_stored_with_pledge_values_its_evidence_does_not_commit_to() {
    setup_logger();
    let output_member = TestAddress::new("2");
    let captured = Arc::new(Mutex::new(None::<(TestAddress, ForeignProposal)>));
    let mut test = Test::builder()
        .add_committee(0, vec!["1"])
        .add_committee(1, vec!["2"])
        .with_message_filter(Box::new({
            let output_member = output_member.clone();
            let captured = captured.clone();
            move |from, to, msg| match msg {
                HotstuffMessage::ForeignProposal(msg) if *to == output_member => {
                    captured
                        .lock()
                        .unwrap()
                        .get_or_insert_with(|| (from.clone(), (*msg.proposal).clone()));
                    false
                },
                _ => true,
            }
        }))
        .start()
        .await;

    let inputs = test.create_substates_on_vns(TestVnDestination::Committee(0), 1);
    let outputs = test.build_outputs_for_committee(1, 1);
    let tx = build_transaction_from(
        Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs.iter().cloned().map(|i| i.into()))
            .build_and_seal(&PrivateKey::default()),
    );
    test.create_execution_at_destination_for_transaction(
        TestVnDestination::All,
        &tx,
        Decision::Commit,
        5,
        inputs
            .into_iter()
            .map(|input| (input.substate_id().clone(), SubstateLockType::Write))
            .collect(),
        outputs,
    );
    test.send_transaction_to_destination(TestVnDestination::All, tx.clone())
        .await;
    test.start_epoch(Epoch(1)).await;

    let (sender, genuine) = timeout(Duration::from_secs(60), async {
        loop {
            if let Some(captured) = captured.lock().unwrap().clone() {
                return captured;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .expect("committee 0 never sent a foreign proposal");
    assert!(!genuine.block_pledge().is_empty());
    let block_id = genuine.calculate_block_id();
    let tx_inbound = test.get_validator(&output_member).tx_inbound_message.clone();
    let is_stored = || {
        test.get_validator(&output_member)
            .state_store()
            .with_read_tx(|tx| ForeignProposalRecord::record_exists(tx, &block_id))
            .unwrap()
    };

    tx_inbound
        .send((
            sender.clone(),
            HotstuffMessage::ForeignProposal(with_substituted_pledge_values(&genuine).into()),
        ))
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;
    assert!(!is_stored(), "stored a foreign proposal with substituted pledge values");

    // The substituted copy leaves nothing behind that stops the genuine proposal from being accepted
    tx_inbound
        .send((sender, HotstuffMessage::ForeignProposal(genuine.into())))
        .await
        .unwrap();
    sleep(Duration::from_secs(2)).await;
    assert!(is_stored());

    test.stop();
}

#[tokio::test]
async fn it_rejects_a_shard_group_with_invalid_bounds() {
    let Setup { epoch_manager, .. } = setup().await;

    let proposal = proposal("b1", tari_sidechain::ShardGroup {
        start: 10,
        end_inclusive: 5,
    });

    let err = resolve_foreign_committee(&epoch_manager, &proposal).await.unwrap_err();
    assert!(
        matches!(
            err,
            HotStuffError::ProposalValidationError(ProposalValidationError::InvalidShardGroup { .. })
        ),
        "unexpected error: {err}"
    );
}
