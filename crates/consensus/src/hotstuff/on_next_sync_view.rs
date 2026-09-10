//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use log::*;
use ootle_byte_type::ToByteType;
use tari_consensus_types::{
    HighPc,
    LastSentNewView,
    LastSentVote,
    ProposalCertificate,
    TimeoutVote,
    TimeoutVoteMessage,
    ValidatorSignatureBytes,
};
use tari_ootle_common_types::{Epoch, NodeHeight, committee::Committee, displayable::Displayable, optional::Optional};
use tari_ootle_storage::{StateStore, consensus_models::BookkeepingModel};

use crate::{
    hotstuff::HotStuffError,
    messages::{HotstuffMessage, NewViewMessage},
    traits::{CertificateStore, ConsensusSpec, LeaderStrategy, OutboundMessaging, ValidatorSignerService},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_next_sync_view";

pub struct OnNextSyncViewHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    leader_strategy: TConsensusSpec::LeaderStrategy,
    signer_service: TConsensusSpec::SignerService,
    last_sent_new_view: Option<(Epoch, NodeHeight)>,
}

impl<TConsensusSpec: ConsensusSpec> OnNextSyncViewHandler<TConsensusSpec> {
    pub fn new(
        store: TConsensusSpec::StateStore,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
        leader_strategy: TConsensusSpec::LeaderStrategy,
        signer_service: TConsensusSpec::SignerService,
    ) -> Self {
        Self {
            store,
            outbound_messaging,
            leader_strategy,
            signer_service,
            last_sent_new_view: None,
        }
    }

    pub async fn handle(
        &mut self,
        epoch: Epoch,
        current_height: NodeHeight,
        local_committee: &Committee<TConsensusSpec::Addr>,
    ) -> Result<(), HotStuffError> {
        let (next_leader, high_pc, last_sent_vote, timeout_height) = self.store.with_read_tx(|tx| {
            // The leader, that is supposed to propose the next block, timed out. Current height is the highest seen
            // view, +1 is the next leader that failed, +2 is the next leader that should propose
            let mut timeout_height = current_height + NodeHeight(2);

            // If we leader failure more than once in a row, propose the next higher view
            if let Some((nv_epoch, last_sent_new_view)) = self.last_sent_new_view &&
                nv_epoch == epoch &&
                last_sent_new_view >= timeout_height
            {
                timeout_height = last_sent_new_view + NodeHeight(1);
            }
            // Skipping the next height since the leader failed to propose
            let (next_leader, _) = self.leader_strategy.get_leader(local_committee, timeout_height);
            let high_pc = HighPc::get(tx, epoch)?;
            let high_pc = ProposalCertificate::get(tx, epoch, high_pc.id())?;
            let last_sent_vote = LastSentVote::get(tx, epoch)
                .optional()?
                .filter(|vote| high_pc.height() < vote.block_height());

            Ok::<_, HotStuffError>((next_leader, high_pc, last_sent_vote, timeout_height))
        })?;

        info!(
            target: LOG_TARGET,
            "🌟 Send NEWVIEW to {} {} HighPC: {} Vote[{}]",
            next_leader,
            timeout_height,
            high_pc,
            last_sent_vote.display(),
        );

        let msg = TimeoutVoteMessage {
            epoch: high_pc.epoch(),
            height: timeout_height,
        };

        let signature = self.signer_service.sign(&msg);
        let signature = ValidatorSignatureBytes::new(
            self.signer_service.public_key().to_byte_type(),
            signature.to_byte_type(),
        );

        let message = NewViewMessage {
            high_pc,
            last_vote: last_sent_vote.map(|vote| vote.vote),
            timeout: TimeoutVote {
                epoch,
                height: timeout_height,
                signature,
            },
        };

        self.outbound_messaging
            .send(next_leader.clone(), HotstuffMessage::new_newview(message))
            .await?;

        self.last_sent_new_view = Some((epoch, timeout_height));
        self.store.with_write_tx(|tx| {
            LastSentNewView {
                epoch,
                height: timeout_height,
            }
            .set(tx)
        })?;

        Ok(())
    }
}
