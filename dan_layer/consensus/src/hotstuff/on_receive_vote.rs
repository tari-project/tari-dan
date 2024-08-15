//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_dan_common_types::committee::CommitteeInfo;

use super::vote_receiver::VoteReceiver;
use crate::{hotstuff::error::HotStuffError, messages::VoteMessage, traits::ConsensusSpec};

pub struct OnReceiveVoteHandler<TConsensusSpec: ConsensusSpec> {
    vote_receiver: VoteReceiver<TConsensusSpec>,
}

impl<TConsensusSpec> OnReceiveVoteHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(vote_receiver: VoteReceiver<TConsensusSpec>) -> Self {
        Self { vote_receiver }
    }

    pub async fn handle(
        &self,
        from: TConsensusSpec::Addr,
        message: VoteMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        self.vote_receiver
            .handle(from, message, true, local_committee_info)
            .await
    }
}
