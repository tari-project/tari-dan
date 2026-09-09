//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet, hash_map::Entry},
    time::{Duration, Instant},
};

use anyhow::anyhow;
use log::*;
use tari_consensus_types::{BlockId, ProposalCertificate};
use tari_epoch_manager::EpochManagerReader;
use tari_ootle_common_types::{Epoch, NodeAddressable, ShardGroup, committee::CommitteeInfo, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{
        Block,
        CommandOrHash,
        CommandsCommitProof,
        ForeignProposal,
        ForeignProposalRecord,
        ForeignProposalStatus,
    },
};

use crate::{
    bounded_spawn::BoundedSpawn,
    hotstuff::{
        ProposalValidationError,
        commit_proofs::generate_block_commit_proof,
        error::HotStuffError,
        pacemaker_handle::PaceMakerHandle,
    },
    messages::{
        ForeignProposalMessage,
        ForeignProposalNotificationMessage,
        ForeignProposalRequestMessage,
        HotstuffMessage,
    },
    tracing::TraceTimer,
    traits::{ConsensusSpec, OutboundMessaging},
};

const LOG_TARGET: &str = "tari::ootle::consensus::hotstuff::on_receive_foreign_proposal";

pub struct OnReceiveForeignProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    pacemaker: PaceMakerHandle,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    pending_requests: PendingRequests<TConsensusSpec::Addr>,
    bounded_spawner: BoundedSpawn,
}

impl<TConsensusSpec> OnReceiveForeignProposalHandler<TConsensusSpec>
where TConsensusSpec: ConsensusSpec
{
    pub fn new(
        store: TConsensusSpec::StateStore,
        epoch_manager: TConsensusSpec::EpochManager,
        pacemaker: PaceMakerHandle,
        outbound_messaging: TConsensusSpec::OutboundMessaging,
    ) -> Self {
        Self {
            store,
            epoch_manager,
            pacemaker,
            outbound_messaging,
            pending_requests: PendingRequests::new(),
            bounded_spawner: BoundedSpawn::new(20),
        }
    }

    pub async fn handle_received(
        &mut self,
        message: ForeignProposalMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveForeignProposal");
        let mut proposal = ForeignProposalRecord::from(message);

        let block_id = *proposal.block_id();
        if self.store.with_read_tx(|tx| proposal.exists(tx))? {
            // This is expected behaviour, we may receive the same foreign proposal multiple times
            debug!(
                target: LOG_TARGET,
                "FOREIGN PROPOSAL: Already received proposal for block {}",
                block_id
            );
            self.pending_requests.remove(&block_id);
            return Ok(());
        }

        self.store.with_write_tx(|tx| {
            if let Err(err) = self.validate_and_save(tx, &proposal, local_committee_info) {
                error!(target: LOG_TARGET, "❌ Error validating and saving foreign proposal: {}", err);
                // Should not cause consensus to crash and should commit the Invalid proposal status
                proposal.save(tx)?;
                proposal.update_status(tx, ForeignProposalStatus::Invalid, None)?;
                // TODO: reattempt from different node? and then abort on persistent failure
                // If we miss a foreign proposal, we want to implement the ability to request it - so we could just rely
                // on that functionality without doing anything extra here
                return Ok(());
            }
            Ok::<_, HotStuffError>(())
        })?;

        self.pending_requests.remove(&block_id);

        // Foreign proposals to propose
        self.pacemaker.beat();
        Ok(())
    }

    pub async fn handle_notification_received(
        &mut self,
        from: TConsensusSpec::Addr,
        current_epoch: Epoch,
        message: ForeignProposalNotificationMessage,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        debug!(
            target: LOG_TARGET,
            "🌐 Receive FOREIGN PROPOSAL NOTIFICATION from {} for block {}",
            from,
            message.block_id,
        );
        if self.pending_requests.contains(&message.block_id) {
            info!(
                target: LOG_TARGET,
                "🌐 FOREIGN PROPOSAL: Already requested block {}. Ignoring.",
                message.block_id,
            );
            return Ok(());
        }
        if self
            .store
            .with_read_tx(|tx| ForeignProposalRecord::record_exists(tx, &message.block_id))?
        {
            // This is expected behaviour, we may receive the same foreign proposal notification multiple times
            debug!(
                target: LOG_TARGET,
                "FOREIGN PROPOSAL: Already received proposal for block {}",
                message.block_id,
            );
            return Ok(());
        }

        // The notification is gossiped on a single network-wide topic, so every validator receives it. Only fetch the
        // foreign proposal if our shard group is in the intended audience.
        if !message.shard_groups.contains(&local_committee_info.shard_group()) {
            debug!(
                target: LOG_TARGET,
                "🌐 FOREIGN PROPOSAL: notification for block {} does not target our shard group {}. Ignoring.",
                message.block_id,
                local_committee_info.shard_group(),
            );
            return Ok(());
        }

        // Check if the source is in a foreign committee
        let Some(foreign_committee_info) = self
            .epoch_manager
            .get_committee_info_by_validator_address(message.epoch, &from)
            .await
            .optional()?
        else {
            warn!(
                target: LOG_TARGET,
                "❌ FOREIGN PROPOSAL: notification for block {} from {} who is not a registered validator for epoch {}. \
                 Ignoring.",
                message.block_id,
                from,
                message.epoch,
            );
            return Ok(());
        };

        if local_committee_info.shard_group() == foreign_committee_info.shard_group() {
            warn!(
                target: LOG_TARGET,
                "❓️ FOREIGN PROPOSAL: Received foreign proposal notification from a validator in the same shard group. Ignoring."
            );
            return Ok(());
        }

        let selected = self
            .epoch_manager
            .get_random_committee_member(
                current_epoch,
                Some(foreign_committee_info.shard_group()),
                Default::default(),
            )
            .await?;

        info!(
            target: LOG_TARGET,
            "🌐 REQUEST foreign proposal {} for block {} from {}",
            foreign_committee_info.shard_group(),
            message.block_id,
            selected,
        );
        self.outbound_messaging
            .send(
                selected.address.clone(),
                HotstuffMessage::ForeignProposalRequest(ForeignProposalRequestMessage::ByBlockId {
                    block_id: message.block_id,
                    for_shard_group: local_committee_info.shard_group(),
                    epoch: message.epoch,
                }),
            )
            .await?;

        self.pending_requests
            .insert(selected.address.clone(), message.block_id, foreign_committee_info);

        Ok(())
    }

    pub async fn handle_requested(
        &mut self,
        from: TConsensusSpec::Addr,
        message: ForeignProposalRequestMessage,
    ) -> Result<(), HotStuffError> {
        let store = self.store.clone();
        let outbound_messaging = self.outbound_messaging.clone();

        // Spawn: Dont block consensus when processing requests.
        if self
            .bounded_spawner
            .try_spawn({
                let from = from.clone();
                async move {
                    let _timer = TraceTimer::debug(LOG_TARGET, "OnReceiveForeignProposalRequest");
                    if let Err(err) = Self::handle_requested_task(store, outbound_messaging, from, message).await {
                        error!(target: LOG_TARGET, "Error handling requested foreign proposal: {}", err);
                    }
                }
            })
            .is_err()
        {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: too many concurrent foreign proposal requests, dropping request from {}",
                from
            );
        }

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    async fn handle_requested_task(
        store: TConsensusSpec::StateStore,
        mut outbound_messaging: TConsensusSpec::OutboundMessaging,
        from: TConsensusSpec::Addr,
        message: ForeignProposalRequestMessage,
    ) -> Result<(), HotStuffError> {
        match message {
            ForeignProposalRequestMessage::ByBlockId {
                block_id,
                for_shard_group,
                ..
            } => {
                info!(
                    target: LOG_TARGET,
                    "🌐 HANDLE foreign proposal request from {} for {}",
                    for_shard_group,
                    block_id,
                );
                let Some(proposal) = store.with_read_tx(|tx| {
                    let Some(block) = Block::get(tx, &block_id).optional()? else {
                        return Ok(None);
                    };
                    let commit_qc = block.get_commit_qc(tx)?;
                    let block_pledge = block.get_block_pledge(tx, for_shard_group)?;

                    let commit_proof = generate_transaction_commands_commit_proof_for_shard_group(
                        tx,
                        &block,
                        &commit_qc,
                        for_shard_group,
                    )?;
                    let proposal = ForeignProposal::new(commit_proof, block_pledge);
                    Ok::<_, HotStuffError>(Some(proposal))
                })?
                else {
                    warn!(
                        target: LOG_TARGET,
                        "FOREIGN PROPOSAL[{}]: Requested block {} not found. Ignoring.",
                        for_shard_group,
                        block_id,
                    );
                    return Ok(());
                };

                info!(
                    target: LOG_TARGET,
                    "🌐 FOREIGN PROPOSAL REPLY to {} foreign proposal {} with {} pledge(s).",
                    for_shard_group,
                    proposal.calculate_block_id(),
                    proposal.block_pledge().len(),
                );

                outbound_messaging
                    .send(from, HotstuffMessage::ForeignProposal(proposal.into()))
                    .await?;
            },
        }

        Ok(())
    }

    pub async fn handle_timed_out_requests(
        &mut self,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        const TIMEOUT: Duration = Duration::from_secs(30);
        let timed_out = self
            .pending_requests
            .drain_timed_out(TIMEOUT)
            .take(10)
            .collect::<Vec<_>>();
        if !timed_out.is_empty() {
            info!(
                target: LOG_TARGET,
                "🌐 FOREIGN PROPOSAL: {} request(s) timed out",
                timed_out.len(),
            );
        }
        for (block_id, requests) in timed_out {
            info!(
                target: LOG_TARGET,
                "🌐 FOREIGN PROPOSAL: Request for block {} timed out (previously sent to {} unique peers). Retrying...",
                block_id,
                requests.num_unique_peers()
            );

            if self
                .store
                .with_read_tx(|tx| ForeignProposalRecord::record_exists(tx, &block_id))?
            {
                // This is expected behaviour, we may receive the same foreign proposal notification multiple times
                debug!(
                    target: LOG_TARGET,
                    "FOREIGN PROPOSAL: Already received proposal for block {}",
                    block_id,
                );
                return Ok(());
            }

            if requests.num_unique_peers() >= local_committee_info.num_shard_group_members() as usize {
                warn!(
                    target: LOG_TARGET,
                    "🌐 FOREIGN PROPOSAL: All validators in shard group {} have been requested for block {}. \
                     Aborting further requests.",
                    requests.shard_group(),
                    block_id,
                );
                // If a FP is never received + proposed by any local member, the transaction will TIMEOUT and ABORT
                return Ok(());
            }

            let shard_group = requests.shard_group();
            let epoch = requests.epoch();

            let selected = self
                .epoch_manager
                .get_random_committee_member(requests.epoch(), Some(requests.shard_group()), requests.peers)
                .await?;

            info!(
                target: LOG_TARGET,
                "🌐 REQUEST foreign proposal {} for block {} from {}",
                shard_group,
                block_id,
                selected,
            );
            self.outbound_messaging
                .send(
                    selected.address.clone(),
                    HotstuffMessage::ForeignProposalRequest(ForeignProposalRequestMessage::ByBlockId {
                        block_id,
                        for_shard_group: local_committee_info.shard_group(),
                        epoch,
                    }),
                )
                .await?;

            self.pending_requests
                .insert(selected.address.clone(), block_id, requests.committee_info)
        }

        Ok(())
    }

    pub fn validate_and_save(
        &self,
        tx: &mut <TConsensusSpec::StateStore as StateStore>::WriteTransaction<'_>,
        proposal: &ForeignProposalRecord,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), HotStuffError> {
        if let Err(err) = self.validate_foreign_proposal(proposal.proposal(), local_committee_info) {
            // TODO: handle this case. Perhaps, by aborting all transactions that are affected by this block (we known
            // the justify QC is valid)
            warn!(
                target: LOG_TARGET,
                "⚠️❌ FOREIGN PROPOSAL: Invalid proposal: {}. Ignoring {}.",
                err,
                proposal,
            );
            return Err(err.into());
        }

        info!(
            target: LOG_TARGET,
            "🧩 Receive FOREIGN PROPOSAL {}",
            proposal
        );

        proposal.save(tx)?;

        Ok(())
    }

    fn validate_foreign_proposal(
        &self,
        proposal: &ForeignProposal,
        local_committee_info: &CommitteeInfo,
    ) -> Result<(), ProposalValidationError> {
        // TODO: validations specific to the foreign proposal. General block validations (signature etc) are already
        //       performed in on_message_validate. These should be in the validator helper module.
        let epoch = local_committee_info.epoch();
        // Allow one epoch behind as Prepare/Accept rounds may have been conducted in the previous/subsequent epoch
        // before/after epoch end
        let epoch_range = epoch.saturating_sub(Epoch(1))..=epoch + Epoch(1);

        if !epoch_range.contains(&proposal.epoch()) {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal epoch: {}. Current epoch: {}",
                proposal.epoch(),
                local_committee_info.epoch(),
            );
            return Err(ProposalValidationError::ForeignProposalInvalid {
                block_id: proposal.calculate_block_id(),
                shard_group: proposal.shard_group_unchecked(),
                details: anyhow!(
                    "Foreign node proposal epoch is not within range of the current epoch. Current epoch: {}, block \
                     epoch: {}",
                    local_committee_info.epoch(),
                    proposal.epoch(),
                ),
            });
        }

        validate_evidence_and_pledges_match(proposal, local_committee_info.shard_group())?;

        Ok(())
    }
}

fn validate_evidence_and_pledges_match(
    proposal: &ForeignProposal,
    local_shard_group: ShardGroup,
) -> Result<(), ProposalValidationError> {
    let foreign_shard_group = proposal.shard_group_checked().ok_or_else(|| {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal: foreign shard group does not match the local shard group. Local Shard Group: {}, Foreign Shard Group: {}",
                local_shard_group,
                proposal.shard_group_unchecked(),
            );
            ProposalValidationError::ForeignProposalInvalid {
                block_id: proposal.calculate_block_id(),
                shard_group: proposal.shard_group_unchecked(),
                details: anyhow!("Invalid shard group bounds")
            }
        })?;
    // TODO: any error will** result in transactions that never resolve.
    // ** unless the foreign shard sends it again with the correct evidence and pledges
    // Possible ways to handle this:
    // - Send a message to the foreign shard to request the pledges again (but why would they return the correct pledges
    //   this time?)
    // - Immediately ABORT all transactions with invalid pledges in the block - this is the safest option
    let mut num_applicable = 0usize;
    for (is_local_accept, atom) in proposal.full_commands_iter().filter_map(|cmd| {
        cmd.local_prepare()
            // The foreign committee may have sent us this block for other transactions that are applicable to us
            // not for this output-only LocalPrepare
            .filter(|atom| !atom.evidence.is_committee_output_only(proposal.shard_group_unchecked()))
            .map(|atom| (false, atom))
            .or_else(|| cmd.local_accept().map(|atom| (true, atom)))
    }) {
        if atom.decision.is_abort() || !atom.evidence.has(&local_shard_group) {
            continue;
        }

        num_applicable += 1;

        // If the local node is involved in inputs (i.e not output-only), we already have the input pledges from the
        // prepare phase and so do not need them to be resent.
        let dont_need_input_pledges = is_local_accept &&
            (!atom.evidence.is_committee_output_only(local_shard_group) ||
                atom.evidence.is_committee_output_only(foreign_shard_group));
        if dont_need_input_pledges {
            // CASE: if we're an input shard group, and we're receiving a local accept, we do not require input pledges
            debug!(
                target: LOG_TARGET,
                "FOREIGN PROPOSAL: Input pledges not required for LocalAccept"
            );
            continue;
        }

        debug!(
            target: LOG_TARGET,
            "🧩 FOREIGN PROPOSAL: Check transaction {} pledges from {} - local_shard_group: {}, is_local_accept: {}, local-output-only: {}",
            atom.id,
            foreign_shard_group,
            local_shard_group,
            is_local_accept,
            atom.evidence.is_committee_output_only(local_shard_group),
        );

        let shard_group_evidence =
            atom.evidence
                .get(&foreign_shard_group)
                .ok_or_else(|| ProposalValidationError::ForeignProposalInvalid {
                    block_id: proposal.calculate_block_id(),
                    shard_group: proposal.shard_group_unchecked(),
                    details: anyhow!(
                        "InvalidPledge: Atom({}) evidence from foreign shard group does not contain evidence for that \
                         shard group",
                        atom.id
                    ),
                })?;

        if !proposal
            .block_pledge()
            .has_all_input_substate_values_for(shard_group_evidence)
        {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal: some input pledges for {}({}) are missing. Local Shard Group: {}, All Pledges: {}",
                if is_local_accept { "LocalAccept" } else { "LocalPrepare" },
                atom.id,
                local_shard_group,
                proposal.block_pledge(),
            );
            return Err(ProposalValidationError::ForeignProposalInvalid {
                block_id: proposal.calculate_block_id(),
                shard_group: foreign_shard_group,
                details: anyhow!(
                    "input pledges are missing one or more substate values for transaction {}",
                    atom.id
                ),
            });
        }
    }

    info!(
        target: LOG_TARGET,
        "🧩 FOREIGN PROPOSAL: OK - {} of {} command(s) apply in {}",
        num_applicable,
        proposal.full_commands_iter().count(),
        proposal,
    );

    Ok(())
}

fn generate_transaction_commands_commit_proof_for_shard_group<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    committed_block: &Block,
    commit_qc: &ProposalCertificate,
    for_shard_group: ShardGroup,
) -> Result<CommandsCommitProof, HotStuffError> {
    let _timer = TraceTimer::info(LOG_TARGET, "generate_transaction_commands_commit_proof_for_shard_group");
    let applicable_commands = committed_block.commands().iter().map(|cmd| {
        let is_involved_local_prepare_with_inputs = cmd
            .local_prepare()
            .map(|atom| atom.evidence.has_inputs(for_shard_group))
            .unwrap_or(false);

        let is_involved_local_accept = cmd
            .local_accept()
            .map(|atom| atom.evidence.has(&for_shard_group))
            .unwrap_or(false);

        if is_involved_local_prepare_with_inputs || is_involved_local_accept {
            CommandOrHash::Command(cmd.clone())
        } else {
            CommandOrHash::Hash(cmd.hash())
        }
    });

    let proof = generate_block_commit_proof(tx, commit_qc, committed_block)?;
    let command_commit_proof = CommandsCommitProof::new_latest(applicable_commands.collect(), proof);
    Ok(command_commit_proof)
}

struct PendingRequests<TAddr> {
    pending: HashMap<BlockId, ForeignRequests<TAddr>>,
}

impl<TAddr: NodeAddressable> PendingRequests<TAddr> {
    pub(self) fn new() -> Self {
        Self {
            pending: HashMap::new(),
        }
    }

    pub(self) fn contains(&self, block_id: &BlockId) -> bool {
        self.pending.contains_key(block_id)
    }

    pub(self) fn insert(&mut self, address: TAddr, block_id: BlockId, committee_info: CommitteeInfo) {
        match self.pending.entry(block_id) {
            Entry::Occupied(occupied) => {
                let entry = occupied.into_mut();
                entry.peers.insert(address);
                entry.at = Instant::now();
            },
            Entry::Vacant(vacant) => {
                let mut peers = HashSet::new();
                peers.insert(address);
                vacant.insert(ForeignRequests {
                    peers,
                    committee_info,
                    at: Instant::now(),
                });
            },
        }
    }

    pub(self) fn remove(&mut self, block_id: &BlockId) -> Option<ForeignRequests<TAddr>> {
        let item = self.pending.remove(block_id);
        if self.pending.capacity() >= 1000 {
            self.pending.shrink_to_fit();
        }
        item
    }

    pub(self) fn drain_timed_out(
        &mut self,
        timeout: Duration,
    ) -> impl Iterator<Item = (BlockId, ForeignRequests<TAddr>)> + '_ {
        self.pending.extract_if(move |_, reqs| reqs.at.elapsed() >= timeout)
    }
}

struct ForeignRequests<TAddr> {
    pub peers: HashSet<TAddr>,
    pub committee_info: CommitteeInfo,
    pub at: Instant,
}

impl<TAddr> ForeignRequests<TAddr> {
    pub fn epoch(&self) -> Epoch {
        self.committee_info.epoch()
    }

    pub fn shard_group(&self) -> ShardGroup {
        self.committee_info.shard_group()
    }

    pub fn num_unique_peers(&self) -> usize {
        self.peers.len()
    }
}
