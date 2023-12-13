//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;

use super::vote_receiver::VoteReceiver;
use crate::{hotstuff::error::HotStuffError, messages::VoteMessage, traits::ConsensusSpec};

const LOG_TARGET: &str = "tari::dan::consensus::hotstuff::on_receive_vote";

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    vote_receiver: VoteReceiver<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveVoteHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(vote_receiver: VoteReceiver<TConsensusSpec>) -> Self {
        Self { vote_receiver }
    }

    #[allow(clippy::too_many_lines)]
    pub async fn handle(&self, from: TConsensusSpec::Addr, message: VoteMessage) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🔥 Receive VOTE for node {} from {}", message.block_id, message.signature.public_key,
        );

        self.vote_receiver.handle(from, message, true).await
    }
}
