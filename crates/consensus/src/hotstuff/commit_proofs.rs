//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_common_types::types::CompressedPublicKey;
use tari_consensus_types::ProposalCertificate;
use tari_crypto::{ristretto::RistrettoSecretKey, tari_utilities::ByteArray};
use tari_ootle_common_types::ProtocolVersion;
use tari_ootle_storage::{
    StateStoreReadTransaction,
    consensus_models::{Block, BlockHeader, EndOfEpochCommand},
};
use tari_ootle_transaction::Network;
use tari_sidechain::{
    ChainLink,
    CommandCommitProof,
    CommitProofElement,
    EvictNodeAtom,
    EvictionProof,
    SidechainBlockCommitProof,
    SidechainBlockHeader,
    ValidatorBlockSignature,
    ValidatorQcSignature,
};
use tari_template_lib_types::crypto::SchnorrSignatureBytes;

use crate::hotstuff::HotStuffError;

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::commit_proofs";

pub fn generate_eviction_proofs<'a, TTx, I>(
    tx: &TTx,
    tip_qc: &ProposalCertificate,
    committed_blocks_with_evictions: I,
) -> Result<Vec<EvictionProof>, HotStuffError>
where
    TTx: StateStoreReadTransaction,
    I: IntoIterator<Item = &'a Block>,
    I::IntoIter: Clone,
{
    let evictions_iter = committed_blocks_with_evictions.into_iter();
    let num_evictions = evictions_iter.clone().map(|b| b.all_node_evictions().count()).sum();

    let mut proofs = Vec::with_capacity(num_evictions);
    for block in evictions_iter {
        // First generate a commit proof for the block which is shared by all EvictionProofs
        let block_commit_proof = generate_block_commit_proof(tx, tip_qc, block)?;

        for (idx, command) in block.commands().iter().enumerate() {
            let Some(atom) = command.evict_node() else {
                continue;
            };
            info!(target: LOG_TARGET, "🦶 Generating eviction proof for validator: {atom}");
            let inclusion_proof = block.compute_command_inclusion_proof(idx)?;
            let atom = EvictNodeAtom::new(
                CompressedPublicKey::from_canonical_bytes(atom.public_key.as_bytes()).map_err(|_| {
                    HotStuffError::InvariantError(format!(
                        "EvictNodeAtom RistrettoPublicKey non-canonical bytes for public key, in \
                         generate_eviction_proofs ({:?})",
                        atom.public_key,
                    ))
                })?,
            );
            let commit_command_proof = CommandCommitProof::new(atom, block_commit_proof.clone(), inclusion_proof);
            let proof = EvictionProof::new(commit_command_proof);
            proofs.push(proof);
        }
    }

    Ok(proofs)
}

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

    let mut block = Block::get(tx, &commit_qc.calculate_block_id())?;
    debug!(target: LOG_TARGET, "⚙️ START: generate commit proof {} {} -> {} {}", block.height(), block.id(), committed_block.height(), committed_block.id());
    debug!(target: LOG_TARGET, "⚙️ Adding the commit_qc to the proof: {commit_qc}");
    let network = committed_block.network();
    proof_elements.push(convert_qc_to_proof_element(network, commit_qc)?);
    while block.id() != committed_block.id() {
        // Prevent possibility of endless loop if the IDs never match - which should be impossible.
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

        if block.justifies_parent() {
            // This block justifies the parent, so we add it to the proof
            debug!(target: LOG_TARGET, "⚙️ Add justify: {}", block.justify());
            proof_elements.push(convert_qc_to_proof_element(network, block.justify())?);
            block = block.get_parent(tx)?;
        } else {
            // This block does not justify the parent. We'll add link(s) back until we find the block that is justified
            // by the PC. NOTE: That these blocks are not necessarily dummy blocks, they simply do not propose a new
            // proposal certificate and so are included in the proof as "chain links".
            // Start from the parent, because the QC that justifies this block was added in the justify_parent() == true
            // above.
            let parent_id = *block.parent();
            let qc = block.into_justify();
            block = Block::get(tx, &parent_id)?;
            let qc_block_id = qc.calculate_block_id();
            let qc_id = qc.calculate_id();
            let qc_height = qc.height();

            // let qc_block_id = block.justify().calculate_block_id();
            // let qc_id = block.justify().calculate_id();
            // let qc_height = block.justify().height();

            debug!(target: LOG_TARGET, "⚙️ Start chain links");

            let mut chain_links = vec![];
            // Continue going back in the chain until we find a block that is justified by the QC
            while *block.parent() != qc_block_id && block.id() != committed_block.id() {
                debug!(target: LOG_TARGET, "⚙️ Add chain link: {block} QC: {qc_height} {qc_block_id} {qc_id}");
                chain_links.push(ChainLink {
                    header_hash: block.header().calculate_hash(),
                    parent_id: *block.parent().hash(),
                });

                block = block.get_parent(tx)?;
                if block.height() < qc_height {
                    return Err(HotStuffError::InvariantError(format!(
                        "Block height is less than the height of the QC in generate_block_commit_proof \
                         (block={block}, qc={qc_height} {qc_block_id} {qc_id})",
                    )));
                }
            }

            if block.id() != committed_block.id() {
                debug!(target: LOG_TARGET, "⚙️ Add final chain link: {block} QC: {qc_height} {qc_block_id} {qc_id}");
                chain_links.push(ChainLink {
                    header_hash: block.header().calculate_hash(),
                    parent_id: *block.parent().hash(),
                });
            }

            debug!(target: LOG_TARGET, "⚙️ End of chain links ({} chain link(s))", chain_links.len());
            proof_elements.push(CommitProofElement::ChainLinks(chain_links));
        }
    }

    debug!(target: LOG_TARGET, "⚙️ END of commit proof generation");
    let command_commit_proof = SidechainBlockCommitProof {
        header: convert_block_to_sidechain_block_header(committed_block.header())?,
        proof_elements,
    };

    Ok(command_commit_proof)
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
        metadata_hash: header.calculate_metadata_hash(),
        signature,
        accumulated_data: (*header.accumulated_data()).into(),
    })
}

fn convert_qc_to_proof_element(
    network: Network,
    qc: &ProposalCertificate,
) -> Result<CommitProofElement, HotStuffError> {
    Ok(CommitProofElement::QuorumCertificate(
        tari_sidechain::QuorumCertificate {
            header_hash: *qc.header_hash(),
            parent_id: *qc.parent_id().hash(),
            epoch: qc.epoch().as_u64(),
            height: qc.height().as_u64(),
            protocol_version: ProtocolVersion::at(network, qc.epoch()).as_u32(),
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
        ProposalCertificate,
        ShardGroupAccumulatedData,
        ToSignatureMessage,
        ValidatorSchnorrSignature,
    };
    use tari_crypto::tari_utilities::epoch_time::EpochTime;
    use tari_ootle_common_types::{
        Epoch,
        ExtraData,
        NodeHeight,
        NumPreshards,
        ShardGroup,
        crypto::create_key_pair_from_seed,
    };
    use tari_sidechain::{ProposalVoteMessage, QuorumDecision, ValidatorQcSignature};

    use super::*;

    fn seed_hash(seed: u8) -> FixedHash {
        let arr = [seed; 32];
        FixedHash::new(arr)
    }

    #[test]
    fn it_hashes_the_header_identically_to_sidechain_header() {
        for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1] {
            assert_hashes_identically_to_sidechain_header(protocol_version);
        }
    }

    fn build_header(protocol_version: ProtocolVersion) -> BlockHeader {
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
            NodeHeight(2),
            Epoch(1),
            shard_group,
            Default::default(),
            Default::default(),
            &Default::default(),
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
        for protocol_version in [ProtocolVersion::V0, ProtocolVersion::V1] {
            assert_vote_verifies_in_the_sidechain_crate(protocol_version);
        }
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

    fn assert_hashes_identically_to_sidechain_header(protocol_version: ProtocolVersion) {
        let block = build_header(protocol_version);
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
            command_merkle_root: Default::default(),
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
