//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::CompressedPublicKey;
use tari_consensus_types::ProposalCertificate;
use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_ootle_common_types::optional::Optional;
use tari_ootle_storage::{
    StateStoreReadTransaction,
    consensus_models::{Block, BlockHeader, CommittedBlockProof, EndOfEpochCommand, StateVersionProofSource},
};
use tari_sidechain::{
    ChainLink,
    CommandCommitProof,
    CommitProofElement,
    SidechainBlockCommitProof,
    SidechainBlockHeader,
    ValidatorBlockSignature,
    ValidatorQcSignature,
};
use tari_template_lib_types::crypto::SchnorrSignatureBytes;

use crate::hotstuff::HotStuffError;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::commit_proofs";

pub fn generate_end_of_epoch_commit_proof<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    commit_qc: &ProposalCertificate,
    committed_block: &Block,
) -> Result<CommandCommitProof<EndOfEpochCommand>, HotStuffError> {
    if committed_block.commands().len() != 1 {
        return Err(HotStuffError::InvariantError(format!(
            "End of epoch block must have exactly one command, but found {}",
            committed_block.commands().len()
        )));
    }

    if !committed_block.is_epoch_end() {
        return Err(HotStuffError::InvariantError(format!(
            "Block is not an end-of-epoch block: {committed_block}"
        )));
    }

    // The single command is the EndEpoch command; its atom carries the next epoch's hash. Rebuild the
    // proof command from it so its hash matches the committed block's command merkle root.
    let end_epoch_atom = committed_block
        .commands()
        .iter()
        .find_map(|cmd| cmd.end_epoch())
        .ok_or_else(|| {
            HotStuffError::InvariantError(format!(
                "End-of-epoch block {committed_block} does not contain an EndEpoch command"
            ))
        })?;

    let proof = generate_block_commit_proof(tx, commit_qc, committed_block)?;
    let inclusion_proof = committed_block.compute_command_inclusion_proof(0)?;
    let command_commit_proof = CommandCommitProof::new(
        EndOfEpochCommand::new(*end_epoch_atom.next_epoch_hash()),
        proof,
        inclusion_proof,
    );
    Ok(command_commit_proof)
}

/// The CBOR-encoded [`CommittedBlockProof`] that a state version proof from `source` is served with.
///
/// A [`StateVersionProofSource::Committed`] block's commit proof is the one stored when it committed. A block with no
/// stored commit proof is proven from the block itself, which is possible only while the block is retained.
pub fn state_version_commit_proof<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    source: StateVersionProofSource,
) -> Result<Vec<u8>, HotStuffError> {
    match source {
        StateVersionProofSource::Committed { block_id } => {
            if let Some(commit_proof) = tx.block_commit_proofs_get(&block_id).optional()? {
                return Ok(commit_proof);
            }
            let block = Block::get(tx, &block_id).optional()?.ok_or_else(|| {
                HotStuffError::InvariantError(format!(
                    "No commit proof is stored for block {block_id}, and the block is pruned, so its state versions \
                     cannot be proven"
                ))
            })?;
            let commit_qc = block.get_commit_qc(tx)?;
            let commit_proof = generate_block_commit_proof(tx, &commit_qc, &block)?;
            Ok(CommittedBlockProof::new(commit_proof).to_bytes())
        },
        StateVersionProofSource::Received { commit_proof } => Ok(commit_proof),
    }
}

pub fn generate_block_commit_proof<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    // The QC that caused the block to commit
    commit_qc: &ProposalCertificate,
    // The block that was committed
    committed_block: &Block,
) -> Result<SidechainBlockCommitProof, HotStuffError> {
    let mut proof_elements = Vec::with_capacity(4);

    if committed_block.is_dummy() || committed_block.signature().is_none() {
        return Err(HotStuffError::InvariantError(format!(
            "Commit block is a dummy block or has no signature in generate_block_commit_proof ({committed_block})",
        )));
    }

    // The verifier needs certificates only for the 3-chain that commits `b`: `QC(b'')`, `QC(b')`, `QC(b)`, where
    // each certified block is the parent of the one before it. Every block below `b` is committed by being its
    // ancestor, so the descent from `b` to the committed block is proven by hash links alone. The one exception is
    // a committed block that is the direct parent of `b`: a link chain names only the blocks strictly between its
    // certificate and the header, so that case is proven with `b`'s own justify.
    const NUM_CHAIN_QCS: usize = 3;

    let mut block = Block::get(tx, &commit_qc.calculate_block_id())?;
    debug!(target: LOG_TARGET, "⚙️ START: generate commit proof {} {} -> {} {}", block.height(), block.id(), committed_block.height(), committed_block.id());
    debug!(target: LOG_TARGET, "⚙️ Adding the commit_qc to the proof: {commit_qc}");
    proof_elements.push(convert_qc_to_proof_element(&block, commit_qc)?);
    let mut num_qcs = 1usize;
    while block.id() != committed_block.id() {
        check_not_below_committed(&block, committed_block)?;

        if num_qcs < NUM_CHAIN_QCS || block.parent() == committed_block.id() {
            if !block.justifies_parent() {
                return Err(HotStuffError::InvariantError(format!(
                    "Block {} does not justify its parent {} in generate_block_commit_proof (commit_qc={}, \
                     commit_block={})",
                    block.as_leaf(),
                    block.parent(),
                    commit_qc.calculate_id(),
                    committed_block.as_leaf(),
                )));
            }
            debug!(target: LOG_TARGET, "⚙️ Add justify: {}", block.justify());
            let parent = block.get_parent(tx)?;
            proof_elements.push(convert_qc_to_proof_element(&parent, block.justify())?);
            num_qcs += 1;
            block = parent;
            continue;
        }

        debug!(target: LOG_TARGET, "⚙️ Start chain links");
        let mut chain_links = vec![];
        block = block.get_parent(tx)?;
        while block.id() != committed_block.id() {
            check_not_below_committed(&block, committed_block)?;
            debug!(target: LOG_TARGET, "⚙️ Add chain link: {block}");
            chain_links.push(ChainLink {
                header_hash: block.header().calculate_hash(),
                parent_id: *block.parent().hash(),
            });
            block = block.get_parent(tx)?;
        }
        debug!(target: LOG_TARGET, "⚙️ End of chain links ({} chain link(s))", chain_links.len());
        proof_elements.push(CommitProofElement::ChainLinks(chain_links));
    }

    debug!(target: LOG_TARGET, "⚙️ END of commit proof generation");
    let command_commit_proof = SidechainBlockCommitProof {
        header: convert_block_to_sidechain_block_header(committed_block.header())?,
        proof_elements,
    };

    Ok(command_commit_proof)
}

/// Guards the walk from the commit certificate down to the committed block against a chain that never reaches it.
fn check_not_below_committed(block: &Block, committed_block: &Block) -> Result<(), HotStuffError> {
    if block.height() < committed_block.height() {
        error!(
            target: LOG_TARGET,
            "⚠️ Invariant error: Block height {} is less than the commit block height {} in generate_block_commit_proof ({}, commit_block={})",
            block.height(),
            committed_block.height(),
            block.as_leaf(),
            committed_block.as_leaf()
        );
        return Err(HotStuffError::InvariantError(format!(
            "Block height {} is less than the commit block height {} in generate_block_commit_proof ({}, \
             commit_block={})",
            block.height(),
            committed_block.height(),
            block.as_leaf(),
            committed_block.as_leaf(),
        )));
    }
    Ok(())
}

pub fn convert_block_to_sidechain_block_header(header: &BlockHeader) -> Result<SidechainBlockHeader, HotStuffError> {
    // NOTE: if an invalid signature is not rejected prior to this, an invariant error will be caused by the block
    // proposer.
    let signature = convert_validator_block_signature(header.signature().expect("checked by caller"))?;

    Ok(SidechainBlockHeader {
        network: header.network().as_byte(),
        protocol_version: header.protocol_version().as_u32(),
        parent_id: *header.parent().hash(),
        justify_id: *header.justify_id().hash(),
        height: header.height().as_u64(),
        epoch: header.epoch().as_u64(),
        epoch_hash: *header.epoch_hash(),
        shard_group: tari_sidechain::ShardGroup {
            start: header.shard_group().start().as_u32(),
            end_inclusive: header.shard_group().end().as_u32(),
        },
        proposed_by: CompressedPublicKey::from_canonical_bytes(header.proposed_by().as_bytes()).map_err(|_| {
            HotStuffError::InvariantError(format!(
                "RistrettoPublicKey non-canonical bytes for proposed_by, in convert_block_to_sidechain_block_header \
                 ({})",
                header.proposed_by(),
            ))
        })?,
        state_merkle_root: *header.state_merkle_root(),
        command_merkle_root: *header.command_merkle_root(),
        transaction_merkle_root: header.transaction_merkle_root().copied(),
        metadata_hash: header.calculate_metadata_hash(),
        signature,
        accumulated_data: (*header.accumulated_data()).into(),
    })
}

/// `justified` is the block `qc` justifies, and its header carries the protocol version the certificate's members
/// signed under. A proof may span an activation, so each certificate is versioned by its own block.
fn convert_qc_to_proof_element(
    justified: &Block,
    qc: &ProposalCertificate,
) -> Result<CommitProofElement, HotStuffError> {
    Ok(CommitProofElement::QuorumCertificate(
        tari_sidechain::QuorumCertificate {
            header_hash: *qc.header_hash(),
            parent_id: *qc.parent_id().hash(),
            epoch: qc.epoch().as_u64(),
            height: qc.height().as_u64(),
            protocol_version: justified.header().protocol_version().as_u32(),
            signatures: qc
                .signatures()
                .iter()
                .map(|s| {
                    Ok(ValidatorQcSignature {
                        public_key: CompressedPublicKey::from_canonical_bytes(s.public_key.as_bytes()).map_err(
                            |_| {
                                HotStuffError::InvariantError(format!(
                                    "RistrettoPublicKey non-canonical bytes for public key, in \
                                     convert_qc_to_proof_element ({:?})",
                                    s.public_key,
                                ))
                            },
                        )?,
                        signature: convert_validator_block_signature(&s.signature)?,
                    })
                })
                .collect::<Result<_, HotStuffError>>()?,
            decision: qc.decision(),
        },
    ))
}

fn convert_validator_block_signature(
    signature: &SchnorrSignatureBytes,
) -> Result<ValidatorBlockSignature, HotStuffError> {
    let public_nonce =
        CompressedPublicKey::from_canonical_bytes(signature.public_nonce().as_bytes()).map_err(|_| {
            HotStuffError::InvariantError(format!(
                "RistrettoPublicKey non-canonical bytes for public nonce, in convert_validator_block_signature ({:?})",
                signature.public_nonce(),
            ))
        })?;
    let signature = RistrettoSecretKey::from_canonical_bytes(signature.signature().as_bytes()).map_err(|_| {
        HotStuffError::InvariantError(format!(
            "RistrettoPublicKey non-canonical bytes for signature, in convert_validator_block_signature ({:?})",
            signature.signature(),
        ))
    })?;

    Ok(ValidatorBlockSignature::new(public_nonce, signature))
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::FixedHash;
    use tari_consensus_types::{
        Decision,
        ProposalCertificate,
        ShardGroupAccumulatedData,
        TcId,
        ToSignatureMessage,
        ValidatorSchnorrSignature,
    };
    use tari_crypto::tari_utilities::epoch_time::EpochTime;
    use tari_engine_types::commit_result::AbortReason;
    use tari_ootle_common_types::{
        Epoch,
        ExtraData,
        NodeHeight,
        NumPreshards,
        ProtocolVersion,
        ShardGroup,
        crypto::create_key_pair_from_seed,
    };
    use tari_ootle_storage::{
        StateStore,
        StateStoreWriteTransaction,
        consensus_models::{BlockCommands, Command, LocalOnlyAtom, MultiShardAtom},
    };
    use tari_ootle_transaction::{Network, TransactionId};
    use tari_sidechain::{ProposalVoteMessage, QuorumDecision, ValidatorQcSignature, check_proof_elements};
    use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};

    use super::*;

    fn seed_hash(seed: u8) -> FixedHash {
        let arr = [seed; 32];
        FixedHash::new(arr)
    }

    const NETWORK: Network = Network::LocalNet;

    fn shard_group() -> ShardGroup {
        ShardGroup::all_shards(NumPreshards::P256)
    }

    fn qc_for(block: &Block) -> ProposalCertificate {
        ProposalCertificate::new(
            block.header().calculate_hash(),
            *block.parent(),
            block.height(),
            block.epoch(),
            shard_group(),
            vec![],
            QuorumDecision::Accept,
        )
    }

    /// A signed block whose distinct `seed` keeps its id unique. `justify` is any certificate; the tests choose
    /// whether it certifies the parent.
    fn real_block(parent: &Block, justify: ProposalCertificate, seed: u8) -> Block {
        Block::create(
            NETWORK,
            ProtocolVersion::V0,
            *parent.id(),
            justify,
            None,
            parent.height() + NodeHeight(1),
            parent.epoch(),
            shard_group(),
            Default::default(),
            Default::default(),
            seed_hash(seed),
            0,
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
        .unwrap()
    }

    fn dummy_block(parent: &Block, justify: ProposalCertificate) -> Block {
        let header = BlockHeader::dummy_block(
            NETWORK,
            ProtocolVersion::V0,
            *parent.id(),
            Default::default(),
            parent.height() + NodeHeight(1),
            justify.calculate_id(),
            parent.epoch(),
            shard_group(),
            *parent.header().state_merkle_root(),
            parent.header().timestamp(),
            *parent.header().epoch_hash(),
            *parent.header().accumulated_data(),
            parent.exhaust_burn_rate(),
        );
        Block::new(header, justify, Default::default(), None)
    }

    fn store_with(chain: &[Block]) -> (RocksDbStateStore<String>, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = RocksDbStateStore::open(dir.path().join("db"), DatabaseOptions::default()).unwrap();
        store
            .with_write_tx(|tx| {
                for block in chain {
                    block.insert(tx)?;
                    tx.proposal_certificates_save(block.justify())?;
                }
                Ok::<_, tari_ootle_storage::StorageError>(())
            })
            .unwrap();
        (store, dir)
    }

    /// Runs the proof through the sidechain verifier's structural checks. The certificates carry no signatures,
    /// so a zero quorum threshold exercises everything but signature verification.
    fn assert_verifies(proof: &SidechainBlockCommitProof) {
        check_proof_elements(
            &proof.header,
            &proof.proof_elements,
            &|_| Ok(true),
            QuorumDecision::Accept,
            0,
        )
        .unwrap();
    }

    fn qc_element_ids(proof: &SidechainBlockCommitProof) -> Vec<Vec<FixedHash>> {
        proof
            .proof_elements
            .iter()
            .map(|elem| match elem {
                CommitProofElement::QuorumCertificate(qc) => vec![qc.calculate_justified_block()],
                CommitProofElement::ChainLinks(links) => links.iter().map(|l| l.calc_block_id()).collect(),
            })
            .collect()
    }

    /// Builds the 3-chain `b <- b' <- b''` on top of `base`, each block justifying its parent, and returns the
    /// blocks with the certificate over `b''` that commits `b`.
    fn three_chain(base: &Block, b_justify: ProposalCertificate, seed: u8) -> (Vec<Block>, ProposalCertificate) {
        let b = real_block(base, b_justify, seed);
        let b1 = real_block(&b, qc_for(&b), seed + 1);
        let b2 = real_block(&b1, qc_for(&b1), seed + 2);
        let commit_qc = qc_for(&b2);
        (vec![b, b1, b2], commit_qc)
    }

    #[test]
    fn it_proves_the_3_chain_with_certificates_and_the_rest_with_links() {
        let zero = Block::zero_block(NETWORK, NumPreshards::P256);
        let committed = real_block(&zero, zero.justify().clone(), 1);
        let r1 = real_block(&committed, qc_for(&committed), 2);
        let d1 = dummy_block(&r1, qc_for(&r1));
        let d2 = dummy_block(&d1, qc_for(&r1));
        let (chain, commit_qc) = three_chain(&d2, qc_for(&r1), 3);
        let [b, b1, b2] = chain.as_slice() else { unreachable!() };
        let all = [&zero, &committed, &r1, &d1, &d2, b, b1, b2];
        let (store, _dir) = store_with(&all.map(Clone::clone));

        let proof = store
            .with_read_tx(|tx| generate_block_commit_proof(tx, &commit_qc, &committed))
            .unwrap();

        assert_eq!(qc_element_ids(&proof), vec![
            vec![*b2.id().hash()],
            vec![*b1.id().hash()],
            vec![*b.id().hash()],
            vec![*d2.id().hash(), *d1.id().hash(), *r1.id().hash()],
        ]);
        assert_verifies(&proof);
    }

    #[test]
    fn it_certifies_a_committed_block_that_is_the_direct_parent_of_the_3_chain() {
        let zero = Block::zero_block(NETWORK, NumPreshards::P256);
        let committed = real_block(&zero, zero.justify().clone(), 1);
        let (chain, commit_qc) = three_chain(&committed, qc_for(&committed), 2);
        let [b, b1, b2] = chain.as_slice() else { unreachable!() };
        let (store, _dir) = store_with(&[&zero, &committed, b, b1, b2].map(Clone::clone));

        let proof = store
            .with_read_tx(|tx| generate_block_commit_proof(tx, &commit_qc, &committed))
            .unwrap();

        assert_eq!(qc_element_ids(&proof), vec![
            vec![*b2.id().hash()],
            vec![*b1.id().hash()],
            vec![*b.id().hash()],
            vec![*committed.id().hash()],
        ]);
        assert_verifies(&proof);
    }

    #[test]
    fn it_proves_the_head_of_the_3_chain_with_exactly_three_certificates() {
        let zero = Block::zero_block(NETWORK, NumPreshards::P256);
        let (chain, commit_qc) = three_chain(&zero, zero.justify().clone(), 1);
        let [b, b1, b2] = chain.as_slice() else { unreachable!() };
        let (store, _dir) = store_with(&[&zero, b, b1, b2].map(Clone::clone));

        let proof = store
            .with_read_tx(|tx| generate_block_commit_proof(tx, &commit_qc, b))
            .unwrap();

        assert_eq!(qc_element_ids(&proof), vec![
            vec![*b2.id().hash()],
            vec![*b1.id().hash()],
            vec![*b.id().hash()],
        ]);
        assert_verifies(&proof);
    }

    #[test]
    fn it_hashes_the_header_identically_to_sidechain_header() {
        let finalizing = BlockCommands::init([
            Command::LocalOnly(LocalOnlyAtom {
                id: TransactionId::new([7; 32]),
                decision: Decision::Abort(AbortReason::ExecutionFailure),
                transaction_fee: 0,
                leader_fee: None,
            }),
            Command::AllAccept(MultiShardAtom {
                id: TransactionId::new([8; 32]),
                decision: Decision::Commit,
                evidence: Default::default(),
                transaction_fee: 0,
                leader_fee: None,
            }),
        ])
        .unwrap();
        for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1, ProtocolVersion::V2] {
            for timeout_certificate_id in [None, Some(TcId::from([4u8; 32]))] {
                for commands in [BlockCommands::empty(), finalizing.clone()] {
                    assert_hashes_identically_to_sidechain_header(protocol_version, timeout_certificate_id, &commands);
                }
            }
        }
    }

    fn build_header(
        protocol_version: ProtocolVersion,
        timeout_certificate_id: Option<TcId>,
        commands: &BlockCommands,
    ) -> BlockHeader {
        let parent_id = seed_hash(1).into_array().into();
        let shard_group = ShardGroup::all_shards(NumPreshards::P256);
        let qc1 = ProposalCertificate::new(
            seed_hash(2),
            parent_id,
            NodeHeight(1),
            Epoch(1),
            shard_group,
            vec![],
            QuorumDecision::Accept,
        );

        let qc1_id = qc1.calculate_id();
        let network = Network::LocalNet;
        BlockHeader::create(
            network,
            protocol_version,
            parent_id,
            qc1_id,
            timeout_certificate_id,
            NodeHeight(2),
            Epoch(1),
            shard_group,
            Default::default(),
            Default::default(),
            commands,
            1,
            SchnorrSignatureBytes::zero(),
            EpochTime::now().as_u64(),
            FixedHash::zero(),
            ShardGroupAccumulatedData::default(),
            ExtraData::new(),
        )
        .unwrap()
    }

    #[test]
    fn a_vote_signed_here_verifies_in_the_sidechain_crate() {
        assert_vote_verifies_in_the_sidechain_crate(ProtocolVersion::V0);
    }

    fn assert_vote_verifies_in_the_sidechain_crate(protocol_version: ProtocolVersion) {
        let (secret, public) = create_key_pair_from_seed(5);
        let (nonce, _) = create_key_pair_from_seed(6);
        let block_id = seed_hash(3);
        let (epoch, height) = (7u64, 9u64);
        let decision = QuorumDecision::Accept;

        let message = ProposalVoteMessage::new(protocol_version.as_u32(), &block_id, decision, epoch, height);
        let signature =
            ValidatorSchnorrSignature::sign_with_nonce_and_message(&secret, nonce, message.to_signature_message())
                .expect("signing is infallible for a valid key");

        let qc_signature = ValidatorQcSignature {
            public_key: CompressedPublicKey::from_canonical_bytes(public.as_bytes()).unwrap(),
            signature: ValidatorBlockSignature::new(
                CompressedPublicKey::from_canonical_bytes(signature.get_public_nonce().as_bytes()).unwrap(),
                signature.get_signature().clone(),
            ),
        };

        assert!(qc_signature.verify(protocol_version.as_u32(), &block_id, decision, epoch, height));
        // The version selects the message, so a certificate cannot claim a version its members did not sign under.
        let other_version = protocol_version.as_u32() ^ 1;
        assert!(!qc_signature.verify(other_version, &block_id, decision, epoch, height));
    }

    fn assert_hashes_identically_to_sidechain_header(
        protocol_version: ProtocolVersion,
        timeout_certificate_id: Option<TcId>,
        commands: &BlockCommands,
    ) {
        let block = build_header(protocol_version, timeout_certificate_id, commands);
        let sidechain_header = SidechainBlockHeader {
            network: block.network().as_byte(),
            protocol_version: block.protocol_version().as_u32(),
            parent_id: *block.parent().hash(),
            justify_id: *block.justify_id().hash(),
            height: block.height().as_u64(),
            epoch: block.epoch().as_u64(),
            epoch_hash: Default::default(),
            shard_group: tari_sidechain::ShardGroup {
                start: 1,
                end_inclusive: 256,
            },
            proposed_by: Default::default(),
            state_merkle_root: Default::default(),
            command_merkle_root: *block.command_merkle_root(),
            transaction_merkle_root: block.transaction_merkle_root().copied(),
            signature: ValidatorBlockSignature::new(
                CompressedPublicKey::from_canonical_bytes(block.signature().unwrap().public_nonce().as_bytes())
                    .unwrap(),
                RistrettoSecretKey::from_canonical_bytes(block.signature().unwrap().signature().as_bytes()).unwrap(),
            ),
            accumulated_data: Default::default(),
            metadata_hash: block.calculate_metadata_hash(),
        };

        assert_eq!(sidechain_header.calculate_hash(), block.calculate_hash());
        assert_eq!(sidechain_header.calculate_block_id(), *block.calculate_id().hash());
    }
}
