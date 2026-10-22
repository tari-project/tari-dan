// Copyright 2026 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! Hashes and encodings that validators and stored proofs depend on. A failure here means a consensus hash or wire
//! format changed, which only a network reset allows.

mod support;

use support::{fixture_proof, load_fixture};
use tari_sidechain::{
    BurnClaimProof,
    BurnOutputProof,
    ChainLink,
    Command,
    CompleteClaimBurnProof,
    EndEpochAtom,
    FixedHash,
    MmrInclusionProof,
    OutputHashPreimage,
    ProposalVoteMessage,
    QuorumDecision,
    SidechainBlockCommitProof,
    TransactionAtom,
};

fn hash(n: u8) -> FixedHash {
    FixedHash::from([n; 32])
}

#[test]
fn header_hash_and_block_id() {
    let header = fixture_proof().header;
    assert_eq!(
        header.calculate_hash().to_string(),
        "27dc0822d624a548cb96b3cf8571e5d7fed0df8b50042f10fe66c8e93eb9b728"
    );
    assert_eq!(
        header.calculate_block_id().to_string(),
        "6ea15995e1dae9c6117ee13bf2a907aecf33dd581fc27e83d589eb0bc348485d"
    );
}

#[test]
fn chain_link_and_justified_block_ids() {
    let link = ChainLink {
        header_hash: hash(0x22),
        parent_id: hash(0x33),
    };
    assert_eq!(
        link.calc_block_id().to_string(),
        "566f5cb0b9db5df987eda6aa3ae4856389c2477e5768e1170fc3bd57c5aa751e"
    );
    let proof = fixture_proof();
    assert_eq!(
        proof.last_qc().unwrap().calculate_justified_block().to_string(),
        "6ea15995e1dae9c6117ee13bf2a907aecf33dd581fc27e83d589eb0bc348485d"
    );
}

#[test]
fn proposal_vote_messages() {
    let block_id = hash(0x44);
    assert_eq!(
        ProposalVoteMessage::new(0, &block_id, QuorumDecision::Accept, 5, 6)
            .calculate_hash()
            .to_string(),
        "775fee04e0286b738814b9213e433e5617032c600f0776256739ac99c4215b06"
    );
    assert_eq!(
        ProposalVoteMessage::new(1, &block_id, QuorumDecision::Reject, 5, 6)
            .calculate_hash()
            .to_string(),
        "f9c624551529ec1938c0eddcb27431da534d5f2f470f31ecefb824e6a27e0c43"
    );
}

#[test]
fn command_hashes() {
    let mut atom = vec![0x55u8; 32];
    atom.extend_from_slice(&[0, 1, 2, 3]);
    assert_eq!(
        Command::LocalOnly(TransactionAtom::from_bytes(atom)).hash().to_string(),
        "28a694aabf3f86af37e13512a1d5ef983e8de40940a3a15c8dc36998c9ea494c"
    );
    assert_eq!(
        Command::EndEpoch(EndEpochAtom::new(hash(0x66))).hash().to_string(),
        "0afd368c82369973f60174aac73c22274d507dd98e2a812c95938fceb169977f"
    );
}

#[test]
fn commit_proof_encodings() {
    let proof = fixture_proof();
    let expected_json: serde_json::Value = load_fixture("commit_proof.json");
    assert_eq!(serde_json::to_value(&proof).unwrap(), expected_json);

    let expected_borsh = std::fs::read_to_string("tests/fixtures/commit_proof.borsh.hex").unwrap();
    let expected_borsh = hex::decode(expected_borsh.trim()).unwrap();
    assert_eq!(borsh::to_vec(&proof).unwrap(), expected_borsh);
    assert_eq!(
        borsh::from_slice::<SidechainBlockCommitProof>(&expected_borsh).unwrap(),
        proof
    );
}

#[test]
fn claim_burn_proof_json() {
    let mmr = |n: u8| MmrInclusionProof {
        leaf_index: u64::from(n),
        mmr_size: 7,
        path: vec![hash(n)],
        peaks: vec![hash(n + 1)],
    };
    let claim = CompleteClaimBurnProof {
        claim_proof: BurnClaimProof {
            burn_public_key: Default::default(),
            ownership_proof: Default::default(),
            output_proof: BurnOutputProof {
                block_hash: hash(0x77),
                block_height: 100,
                output: OutputHashPreimage {
                    version: 1,
                    features: vec![1, 2],
                    commitment: Default::default(),
                    rangeproof_hash: hash(0x88),
                    script: vec![3],
                    sender_offset_public_key: Default::default(),
                    metadata_signature: Default::default(),
                    covenant: vec![0],
                    encrypted_data: vec![9; 10],
                    minimum_value_promise: 0,
                },
                normal_output_proof: mmr(1),
                normal_output_mr: hash(0x99),
                block_output_proof: mmr(3),
            },
            value: 12_345,
        },
        encrypted_data: vec![9, 9, 9],
        mined_in_epoch: 42,
    };

    let json = serde_json::to_string(&claim).unwrap();
    assert_eq!(json, include_str!("fixtures/claim_burn_proof.json").trim());
    let parsed: CompleteClaimBurnProof = serde_json::from_str(&json).unwrap();
    assert_eq!(serde_json::to_string(&parsed).unwrap(), json);
}
