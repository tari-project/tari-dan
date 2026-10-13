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
    bounded_spawn::PerKeyBoundedSpawn,
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

/// Number of foreign proposal requests served concurrently. Each one reads a block and sends it on.
const MAX_CONCURRENT_PROPOSAL_REQUESTS: usize = 20;
/// Two in flight lets a peer pipeline without letting one peer's committee starve the others.
const MAX_PROPOSAL_REQUESTS_PER_PEER: usize = 2;
/// How long we wait for a foreign proposal we asked for before asking again.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Notifications a peer may have awaiting a foreign proposal at once. An honest notifier has one outstanding per
/// recently committed foreign block that involves us, each normally answered well within [`REQUEST_TIMEOUT`].
pub const MAX_PENDING_REQUESTS_PER_NOTIFIER: usize = 32;
/// The same deadline the requester applies, so a permit is never held for a response its asker has given up on.
const PROPOSAL_RESPONSE_TIMEOUT: Duration = REQUEST_TIMEOUT;

pub struct OnReceiveForeignProposalHandler<TConsensusSpec: ConsensusSpec> {
    store: TConsensusSpec::StateStore,
    epoch_manager: TConsensusSpec::EpochManager,
    pacemaker: PaceMakerHandle,
    outbound_messaging: TConsensusSpec::OutboundMessaging,
    pending_requests: PendingRequests<TConsensusSpec::Addr>,
    bounded_spawner: PerKeyBoundedSpawn<TConsensusSpec::Addr>,
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
            bounded_spawner: PerKeyBoundedSpawn::new(
                MAX_CONCURRENT_PROPOSAL_REQUESTS,
                MAX_PROPOSAL_REQUESTS_PER_PEER,
                PROPOSAL_RESPONSE_TIMEOUT,
            ),
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

        let is_saved = self.store.with_write_tx(|tx| {
            if let Err(err) = self.validate_and_save(tx, &proposal, local_committee_info) {
                if matches!(
                    err.validation_error(),
                    Some(ProposalValidationError::ForeignPledgesNotCommitted { .. })
                ) {
                    // The pledges travel beside the commit proof, so a bad set says nothing about the block itself.
                    // Nothing is recorded against the block id, so another copy is still accepted. A pending request
                    // for the block is retried with another member of the foreign committee when it times out.
                    warn!(target: LOG_TARGET, "⚠️ Discarding foreign proposal: {}", err);
                    return Ok(false);
                }
                error!(target: LOG_TARGET, "❌ Error validating and saving foreign proposal: {}", err);
                // Should not cause consensus to crash and should commit the Invalid proposal status
                proposal.save(tx)?;
                proposal.update_status(tx, ForeignProposalStatus::Invalid, None)?;
                // TODO: reattempt from different node? and then abort on persistent failure
                // If we miss a foreign proposal, we want to implement the ability to request it - so we could just rely
                // on that functionality without doing anything extra here
                return Ok(true);
            }
            Ok::<_, HotStuffError>(true)
        })?;
        if !is_saved {
            return Ok(());
        }

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

        if !self.pending_requests.insert(
            from.clone(),
            selected.address.clone(),
            message.block_id,
            foreign_committee_info,
        ) {
            debug!(
                target: LOG_TARGET,
                "🌐 FOREIGN PROPOSAL: {} has {} requests outstanding. Block {} is requested once and not retried.",
                from,
                MAX_PENDING_REQUESTS_PER_NOTIFIER,
                message.block_id,
            );
        }

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
            .try_spawn(from.clone(), {
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
                "⚠️ FOREIGN PROPOSAL: no request slot available for {}, dropping the request",
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
        let timed_out = self.pending_requests.drain_timed_out(REQUEST_TIMEOUT);
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
                continue;
            }

            if requests.num_unique_peers() >= requests.committee_info.num_shard_group_members() as usize {
                warn!(
                    target: LOG_TARGET,
                    "🌐 FOREIGN PROPOSAL: All validators in shard group {} have been requested for block {}. \
                     Aborting further requests.",
                    requests.shard_group(),
                    block_id,
                );
                // A transaction left waiting in LocalPrepared on this proposal aborts once its max_epoch has passed
                // (see `TransactionPoolRecord::is_local_prepared_expired_at`)
                continue;
            }

            let shard_group = requests.shard_group();
            let epoch = requests.epoch();

            let selected = self
                .epoch_manager
                .get_random_committee_member(requests.epoch(), Some(requests.shard_group()), requests.peers.clone())
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
                .insert_retry(block_id, requests, selected.address.clone());
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
        // Callers authenticate the proposal against its committee with `check_foreign_proposal` before this runs.
        // TODO: these should be in the validator helper module.
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

pub(super) fn validate_evidence_and_pledges_match(
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
    let protocol_version =
        proposal
            .protocol_version()
            .map_err(|e| ProposalValidationError::ForeignProposalInvalid {
                block_id: proposal.calculate_block_id(),
                shard_group: foreign_shard_group,
                details: e.into(),
            })?;
    // A rejected proposal is never applied. A transaction waiting on it in LocalPrepared aborts once its max_epoch has
    // passed (see `TransactionPoolRecord::is_local_prepared_expired_at`); one that has already sent its LocalAccept
    // keeps waiting for the foreign shard group.
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
            .has_all_input_substate_values_for(protocol_version, shard_group_evidence)
        {
            warn!(
                target: LOG_TARGET,
                "⚠️ FOREIGN PROPOSAL: Invalid proposal: some input pledges for {}({}) are missing or not the committed value. Local Shard Group: {}, All Pledges: {}",
                if is_local_accept { "LocalAccept" } else { "LocalPrepare" },
                atom.id,
                local_shard_group,
                proposal.block_pledge(),
            );
            return Err(ProposalValidationError::ForeignPledgesNotCommitted {
                block_id: proposal.calculate_block_id(),
                shard_group: foreign_shard_group,
                transaction_id: atom.id,
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
            CommandOrHash::Hash(cmd.hash(committed_block.header().protocol_version()))
        }
    });

    let proof = generate_block_commit_proof(tx, commit_qc, committed_block)?;
    let command_commit_proof = CommandsCommitProof::new_latest(applicable_commands.collect(), proof);
    Ok(command_commit_proof)
}

/// Requests for foreign proposals we were notified of and have not yet received. Each entry is charged to the peer
/// whose notification created it, so that no single peer can grow the set beyond
/// [`MAX_PENDING_REQUESTS_PER_NOTIFIER`].
struct PendingRequests<TAddr> {
    pending: HashMap<BlockId, ForeignRequests<TAddr>>,
    num_pending_by_notifier: HashMap<TAddr, usize>,
}

impl<TAddr: NodeAddressable> PendingRequests<TAddr> {
    pub(self) fn new() -> Self {
        Self {
            pending: HashMap::new(),
            num_pending_by_notifier: HashMap::new(),
        }
    }

    pub(self) fn contains(&self, block_id: &BlockId) -> bool {
        self.pending.contains_key(block_id)
    }

    pub(self) fn num_notified_by(&self, notifier: &TAddr) -> usize {
        self.num_pending_by_notifier.get(notifier).copied().unwrap_or(0)
    }

    /// Records a request made in response to a notification from `notified_by` and charges it to that notifier.
    /// Returns `false`, recording nothing, when the notifier already has [`MAX_PENDING_REQUESTS_PER_NOTIFIER`] charged
    /// requests. Charged requests are never evicted: gossip delivers a single copy of each notification, so a
    /// request dropped here is never made again.
    pub(self) fn insert(
        &mut self,
        notified_by: TAddr,
        requested_from: TAddr,
        block_id: BlockId,
        committee_info: CommitteeInfo,
    ) -> bool {
        if let Some(entry) = self.pending.get_mut(&block_id) {
            entry.peers.insert(requested_from);
            entry.at = Instant::now();
            return true;
        }

        if self.num_notified_by(&notified_by) >= MAX_PENDING_REQUESTS_PER_NOTIFIER {
            return false;
        }

        *self.num_pending_by_notifier.entry(notified_by.clone()).or_default() += 1;
        self.pending.insert(block_id, ForeignRequests {
            peers: HashSet::from([requested_from]),
            charged_to: Some(notified_by),
            committee_info,
            at: Instant::now(),
        });
        true
    }

    /// Records the retry of a timed-out request, keeping every peer already asked. A retry is not charged to the
    /// notifier, so a request for a genuine block cannot be crowded out by notifications sent after it. Each entry
    /// leaves once every member of the foreign committee has been asked, which bounds the uncharged entries a
    /// notifier can cause to [`MAX_PENDING_REQUESTS_PER_NOTIFIER`] per committee member.
    pub(self) fn insert_retry(
        &mut self,
        block_id: BlockId,
        mut requests: ForeignRequests<TAddr>,
        requested_from: TAddr,
    ) {
        requests.peers.insert(requested_from);
        requests.charged_to = None;
        requests.at = Instant::now();
        self.pending.insert(block_id, requests);
    }

    pub(self) fn remove(&mut self, block_id: &BlockId) -> Option<ForeignRequests<TAddr>> {
        let item = self.pending.remove(block_id);
        if let Some(notifier) = item.as_ref().and_then(|requests| requests.charged_to.as_ref()) {
            self.release_notifier(notifier);
        }
        if self.pending.capacity() >= 1000 {
            self.pending.shrink_to_fit();
        }
        item
    }

    pub(self) fn drain_timed_out(&mut self, timeout: Duration) -> Vec<(BlockId, ForeignRequests<TAddr>)> {
        let timed_out = self
            .pending
            .extract_if(|_, reqs| reqs.at.elapsed() >= timeout)
            .collect::<Vec<_>>();
        for notifier in timed_out.iter().filter_map(|(_, requests)| requests.charged_to.clone()) {
            self.release_notifier(&notifier);
        }
        timed_out
    }

    fn release_notifier(&mut self, notifier: &TAddr) {
        if let Entry::Occupied(mut count) = self.num_pending_by_notifier.entry(notifier.clone()) {
            *count.get_mut() -= 1;
            if *count.get() == 0 {
                count.remove();
            }
        }
    }
}

struct ForeignRequests<TAddr> {
    pub peers: HashSet<TAddr>,
    /// The notifier whose allowance this request counts against, until it first times out.
    pub charged_to: Option<TAddr>,
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

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::{NumPreshards, VotePower};

    use super::*;

    fn committee_info() -> CommitteeInfo {
        CommitteeInfo::new(
            NumPreshards::P64,
            1,
            2,
            ShardGroup::new(0, 31),
            Epoch(1),
            VotePower::of(1),
        )
    }

    fn block_id(seed: usize) -> BlockId {
        let mut bytes = [0u8; 32];
        bytes[..8].copy_from_slice(&seed.to_le_bytes());
        BlockId::from(bytes)
    }

    #[test]
    fn a_notifier_at_its_cap_cannot_displace_its_earlier_requests() {
        let mut pending = PendingRequests::<String>::new();
        let flooder = "flooder".to_string();
        let num_notifications = MAX_PENDING_REQUESTS_PER_NOTIFIER + 20;
        let recorded = (0..num_notifications)
            .filter(|seed| {
                pending.insert(
                    flooder.clone(),
                    "foreign".to_string(),
                    block_id(*seed),
                    committee_info(),
                )
            })
            .count();

        assert_eq!(recorded, MAX_PENDING_REQUESTS_PER_NOTIFIER);
        assert_eq!(pending.num_notified_by(&flooder), MAX_PENDING_REQUESTS_PER_NOTIFIER);
        assert!(pending.contains(&block_id(0)));
        assert!(!pending.contains(&block_id(num_notifications - 1)));

        // Another notifier has its own allowance
        let other = "other".to_string();
        assert!(pending.insert(
            other.clone(),
            "foreign".to_string(),
            block_id(usize::MAX),
            committee_info(),
        ));
        assert_eq!(pending.num_notified_by(&other), 1);

        pending.remove(&block_id(usize::MAX));
        assert_eq!(pending.num_notified_by(&other), 0);
    }

    #[test]
    fn a_retried_request_is_not_charged_and_keeps_the_peers_already_asked() {
        let mut pending = PendingRequests::<String>::new();
        let notifier = "notifier".to_string();
        for seed in 0..MAX_PENDING_REQUESTS_PER_NOTIFIER {
            pending.insert(
                notifier.clone(),
                "foreign1".to_string(),
                block_id(seed),
                committee_info(),
            );
        }

        let timed_out = pending.drain_timed_out(Duration::ZERO);
        assert_eq!(timed_out.len(), MAX_PENDING_REQUESTS_PER_NOTIFIER);
        assert_eq!(pending.num_notified_by(&notifier), 0);
        for (block_id, requests) in timed_out {
            pending.insert_retry(block_id, requests, "foreign2".to_string());
        }
        assert_eq!(pending.num_notified_by(&notifier), 0);
        assert!(
            pending
                .pending
                .values()
                .all(|requests| requests.num_unique_peers() == 2 && requests.charged_to.is_none())
        );

        // The notifier's allowance is free for new notifications
        assert!(pending.insert(
            notifier.clone(),
            "foreign1".to_string(),
            block_id(usize::MAX),
            committee_info(),
        ));
        assert_eq!(pending.num_notified_by(&notifier), 1);

        pending.remove(&block_id(0));
        assert_eq!(pending.num_notified_by(&notifier), 1);
    }
}
