//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::committee::{Committee, CommitteeShard};
use tari_dan_storage::global::models::ValidatorNode;

use crate::{
    hotstuff::HotStuffError,
    messages::VoteMessage,
    traits::{ConsensusSpec, VoteSignatureService},
};

pub fn check_vote_message<TConsensusSpec: ConsensusSpec>(
    from: &TConsensusSpec::Addr,
    message: &VoteMessage,
    committee: &Committee<TConsensusSpec::Addr>,
    local_committee_shard: &CommitteeShard,
    our_vn: &ValidatorNode<TConsensusSpec::Addr>,
    vote_signature_service: &TConsensusSpec::SignatureService,
) -> Result<(), HotStuffError> {
    // Is a committee member sending us this vote?
    if !committee.contains(&from) {
        return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
            epoch: message.epoch,
            sender: from.to_string(),
            context: "OnReceiveVote".to_string(),
        });
    }

    // Are we the leader for the block being voted for?

    // Feels like duplication here:
    // Get the sender shard, and check that they are in the local committee
    // let sender_vn = self.epoch_manager.get_validator_node(message.epoch, &from).await?;
    // if message.signature.public_key != sender_vn.public_key {
    //     return Err(HotStuffError::RejectingVoteNotSentBySigner {
    //         address: from.to_string(),
    //         signer_public_key: message.signature.public_key.to_string(),
    //     });
    //   }

    // More duplication
    // if !local_committee_shard.includes_substate_address(&sender_vn.shard_key) {
    //     return Err(HotStuffError::ReceivedMessageFromNonCommitteeMember {
    //         epoch: message.epoch,
    //         sender: message.signature.public_key.to_string(),
    //         context: "OnReceiveVote".to_string(),
    //     });
    // }

    if !vote_signature_service.verify(&message.signature, &message.block_id, &message.decision) {
        return Err(HotStuffError::InvalidVoteSignature {
            signer_public_key: message.signature.public_key().to_string(),
        });
    }

    todo!()
}
