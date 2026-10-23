//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Each test proposes one transaction command that differs from what an honest proposer sends in a single way, and
//! asserts the reason a replica gives for refusing it.

use std::{
    collections::HashMap,
    num::NonZeroU64,
    sync::{Arc, Mutex},
    time::Duration,
};

use tari_common_types::types::PrivateKey;
use tari_consensus_types::{Decision, PcId, ProposalCertificate, ShardGroupAccumulatedData};
use tari_crypto::{ristretto::RistrettoPublicKey, tari_utilities::epoch_time::EpochTime};
use tari_engine_types::{
    commit_result::{AbortReason, ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    component::{Component, ComponentBody, ComponentHeader},
    fees::{FeeBreakdown, FeeReceipt, FeeSource},
    substate::{Substate, SubstateDiff, SubstateId, SubstateValue},
    transaction_receipt::{FinalizeOutcome, TransactionReceipt},
};
use tari_ootle_common_types::{
    Epoch,
    ExtraData,
    LockIntent,
    NodeHeight,
    NumPreshards,
    ProtocolVersion,
    ShardGroup,
    SubstateLockType,
    SubstateRequirement,
    SubstateVersion,
    VersionedSubstateId,
    VotePower,
    committee::CommitteeInfo,
};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    StateStoreWriteTransaction,
    StorageError,
    consensus_models::{
        Block,
        BlockCommands,
        BlockHeader,
        Command,
        Evidence,
        InvalidEvidenceReason,
        LeaderFee,
        LocalOnlyAtom,
        LockedEpoch,
        MultiShardAtom,
        NoVoteReason,
        SubstatePledge,
        SubstateRecord,
        SubstateTransition,
        SubstateUpdateBatch,
        TransactionExecution,
        TransactionPoolStage,
        TransactionPoolStatusUpdate,
        TransactionRecord,
        VersionedSubstateIdLockIntent,
        calculate_leader_fee,
    },
};
use tari_ootle_transaction::{Network, Transaction, TransactionId, TransactionIntent};
use tari_sidechain::{FixedHash, QuorumDecision};
use tari_state_store_rocksdb::{DatabaseOptions, RocksDbStateStore};
use tari_template_lib_types::{
    ComponentAddress,
    EntityId,
    ObjectKey,
    SubstateOwnerRule,
    TransactionReceiptAddress,
    constants::TARI_TOKEN,
    crypto::RistrettoPublicKeyBytes,
};
use tempfile::TempDir;

use super::LocalCommandValidator;
use crate::{
    hotstuff::{
        block_change_set::ProposedBlockChangeSet,
        substate_store::PendingSubstateStore,
        transaction_manager::{ConsensusTransactionManager, EvidenceOrExecution, ExecutionMemo, PreparedTransaction},
    },
    traits::{BlockTransactionExecutor, BlockTransactionExecutorError, CertificateStore, UncommittedChainTemplates},
};

const NETWORK: Network = Network::LocalNet;
const NUM_PRESHARDS: NumPreshards = NumPreshards::P256;
const FEE: u64 = 1_900;

type TestStore = RocksDbStateStore<String>;

/// What the scripted engine produces for a transaction.
#[derive(Debug, Clone, Default)]
struct Script {
    commit: bool,
    /// Substates the transaction creates, besides the next version of each input it writes and its receipt.
    new_outputs: Vec<SubstateId>,
    /// Substates the committed diff downs besides the inputs it writes.
    extra_downs: Vec<VersionedSubstateId>,
    /// Substates the execution reports reading at version zero besides the inputs it is given.
    extra_reads: Vec<SubstateId>,
}

/// Stands in for the engine: a transaction executes as its script says, writing every input it is given that is not
/// read-only.
#[derive(Debug, Clone, Default)]
struct ScriptedExecutor {
    scripts: Arc<Mutex<HashMap<TransactionId, Script>>>,
    num_executions: Arc<Mutex<HashMap<TransactionId, usize>>>,
}

impl ScriptedExecutor {
    fn script(&self, transaction_id: TransactionId, script: Script) {
        self.scripts.lock().unwrap().insert(transaction_id, script);
    }

    /// How many times `transaction_id` has been executed.
    fn num_executions(&self, transaction_id: &TransactionId) -> usize {
        self.num_executions
            .lock()
            .unwrap()
            .get(transaction_id)
            .copied()
            .unwrap_or_default()
    }
}

impl BlockTransactionExecutor<TestStore> for ScriptedExecutor {
    fn execute(
        &self,
        transaction: &Transaction,
        _locked_epoch: LockedEpoch,
        _input_fanout: u32,
        resolved_inputs: &HashMap<SubstateRequirement, Substate>,
        _templates: &UncommittedChainTemplates,
    ) -> Result<TransactionExecution, BlockTransactionExecutorError> {
        let id = transaction.calculate_id();
        *self.num_executions.lock().unwrap().entry(id).or_default() += 1;
        let script = self
            .scripts
            .lock()
            .unwrap()
            .get(&id)
            .cloned()
            .unwrap_or_else(|| panic!("no script for transaction {id}"));

        let inputs = resolved_inputs
            .iter()
            .map(|(requirement, substate)| {
                let lock_type = if requirement.substate_id().is_read_only() {
                    SubstateLockType::Read
                } else {
                    SubstateLockType::Write
                };
                VersionedSubstateIdLockIntent::new(
                    VersionedSubstateId::new(requirement.substate_id().clone(), substate.version()),
                    lock_type,
                    requirement.version().is_some(),
                )
                .with_pledged_value(substate.substate_value())
            })
            .chain(script.extra_reads.iter().map(|id| {
                VersionedSubstateIdLockIntent::new(
                    VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO),
                    SubstateLockType::Read,
                    false,
                )
            }))
            .collect::<Vec<_>>();

        if !script.commit {
            let result = execute_result(
                transaction,
                TransactionResult::Reject(RejectReason::execution_failure_unclassified("scripted abort")),
            );
            return Ok(TransactionExecution::new(result, inputs, Vec::new()));
        }

        let receipt = SubstateId::TransactionReceipt(TransactionReceiptAddress::from(id));
        let mut diff = SubstateDiff::new();
        let mut outputs = Vec::new();
        for input in inputs.iter().filter(|input| input.lock_type().is_write()) {
            let id = input.versioned_substate_id();
            diff.down(id.substate_id().clone(), id.version());
            let next = id.to_next_version();
            diff.up(next.substate_id().clone(), component(next.version()));
            outputs.push(next);
        }
        for id in script.extra_downs {
            diff.down(id.substate_id().clone(), id.version());
        }
        for id in script.new_outputs {
            diff.up(id.clone(), component(SubstateVersion::ZERO));
            outputs.push(VersionedSubstateId::new(id, SubstateVersion::ZERO));
        }
        diff.up(
            receipt.clone(),
            Substate::new(SubstateVersion::ZERO, TransactionReceipt {
                outcome: FinalizeOutcome::Commit,
                diff_summary: Default::default(),
                fee_withdrawals: Default::default(),
                events: Default::default(),
                fee_receipt: fee_receipt(),
                epoch: Epoch::zero(),
                intent_commitment: transaction.calculate_intent_commitment(),
            }),
        );
        outputs.push(VersionedSubstateId::new(receipt, SubstateVersion::ZERO));

        let outputs = outputs.into_iter().map(VersionedSubstateIdLockIntent::output).collect();
        let result = execute_result(transaction, TransactionResult::Accept(diff));
        Ok(TransactionExecution::new(result, inputs, outputs))
    }
}

/// A fully-paid fee receipt in which [FEE] is the leader's share, with a 5% burn on top.
fn fee_receipt() -> FeeReceipt {
    let exhaust_burn = FEE / 19;
    let paid = FEE + exhaust_burn;
    let mut cost_breakdown = FeeBreakdown::default();
    cost_breakdown.add(FeeSource::WasmExecution, paid);
    FeeReceipt::builder()
        .with_total_fee_payment(paid)
        .with_total_fees_paid(paid)
        .with_total_fee_overcharge(0)
        .with_cost_breakdown(cost_breakdown)
        .with_exhaust_burn(exhaust_burn)
        .build()
}

fn execute_result(transaction: &Transaction, result: TransactionResult) -> ExecuteResult {
    ExecuteResult {
        finalize: FinalizeResult::new(
            transaction.calculate_id().into_array().into(),
            vec![],
            vec![],
            result,
            fee_receipt(),
        ),
        execution_time: Duration::ZERO,
        execute_epoch: None,
        wasm_execution_points: 1_000,
        native_execution_points: 0,
    }
}

fn component(version: SubstateVersion) -> Substate {
    Substate::new(version, component_value())
}

fn component_value() -> SubstateValue {
    Component {
        header: ComponentHeader {
            template_address: Default::default(),
            owner_rule: SubstateOwnerRule::None,
            access_rules: Default::default(),
            entity_id: EntityId::default(),
        },
        body: ComponentBody {
            state: tari_bor::Value::Null,
        },
    }
    .into()
}

fn tari_token() -> SubstateId {
    TARI_TOKEN.into()
}

/// A component address. A seed must not repeat the object key of another substate the test uses, such as
/// [TARI_TOKEN]'s, because a substate's address is its object key and version.
fn component_id(seed: u8) -> SubstateId {
    ComponentAddress::from_array([seed; ObjectKey::LENGTH]).into()
}

/// A component address in `shard_group`.
fn component_id_in(shard_group: ShardGroup) -> SubstateId {
    (0x20..=u8::MAX)
        .map(component_id)
        .find(|id| {
            let shard = VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO).to_shard(NUM_PRESHARDS);
            shard_group.contains(&shard)
        })
        .expect("some seed falls in the shard group")
}

/// A chain of the zero block, an uncommitted child of it, and the block a replica evaluates extending that child.
struct Replica {
    store: TestStore,
    executor: ScriptedExecutor,
    manager: ConsensusTransactionManager<ScriptedExecutor, TestStore>,
    committee: CommitteeInfo,
    parent: Block,
    /// The stored block a replica evaluates commands in.
    candidate: Block,
    public_key: RistrettoPublicKey,
    _tmp: TempDir,
}

impl Replica {
    /// A replica whose committee is the only one in the network.
    fn new() -> Self {
        Self::in_committee(ShardGroup::all_shards(NUM_PRESHARDS), 1)
    }

    /// The replica speculates a block's executions on `execution_threads` threads before evaluating it.
    fn with_execution_threads(mut self, execution_threads: usize) -> Self {
        self.manager = ConsensusTransactionManager::new(self.executor.clone(), 2160, usize::MAX)
            .with_speculation(execution_threads, self.store.clone());
        self
    }

    /// A replica whose committee covers `shard_group` of a network split into `num_committees`.
    fn in_committee(shard_group: ShardGroup, num_committees: u32) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let store = TestStore::open(tmp.path(), DatabaseOptions::default()).unwrap();
        let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);
        let parent = child(&zero, shard_group, BlockCommands::empty());
        let candidate = child(&parent, shard_group, BlockCommands::empty());
        store
            .with_write_tx(|tx| {
                for block in [&zero, &parent, &candidate] {
                    block.justify().save(tx)?;
                    block.insert(tx)?;
                }
                Ok::<_, StorageError>(())
            })
            .unwrap();
        let executor = ScriptedExecutor::default();
        let replica = Self {
            store,
            manager: ConsensusTransactionManager::new(executor.clone(), 2160, usize::MAX),
            executor,
            committee: CommitteeInfo::new(
                NUM_PRESHARDS,
                1,
                num_committees,
                shard_group,
                Epoch::zero(),
                VotePower::of(1),
            ),
            parent,
            candidate,
            public_key: RistrettoPublicKey::default(),
            _tmp: tmp,
        };
        // Every transaction reads the fee resource
        replica.add_substate(&tari_token());
        replica
    }

    /// Commits version zero of `id` to the store.
    fn add_substate(&self, id: &SubstateId) {
        let shard = VersionedSubstateId::new(id.clone(), SubstateVersion::ZERO).to_shard(NUM_PRESHARDS);
        let mut batch = SubstateUpdateBatch::new(NETWORK, Epoch::zero());
        batch.with_transition(shard, 0).push(SubstateTransition::Up {
            id: id.clone(),
            version: SubstateVersion::ZERO,
            substate_or_hash: component_value().into(),
        });
        self.store
            .with_write_tx(|tx| SubstateRecord::commit_batch(tx, batch))
            .unwrap();
    }

    /// Pools a new transaction that writes `inputs` and executes as `script` says.
    /// The transaction's receipt is created in this replica's shard group, so the receipt alone never makes it
    /// cross-shard.
    fn pool_transaction(&self, seed: u64, inputs: &[&SubstateId], script: Script) -> TransactionId {
        let transaction = (0..)
            .map(|attempt| {
                let mut builder = Transaction::builder_localnet(Epoch(100));
                for input in inputs {
                    builder = builder.add_input(SubstateRequirement::unversioned((*input).clone()));
                }
                builder.build_and_seal(&PrivateKey::from(seed * 1_000 + attempt))
            })
            .find(|transaction| {
                let receipt =
                    SubstateId::TransactionReceipt(TransactionReceiptAddress::from(transaction.calculate_id()));
                self.committee.includes_substate_id(&receipt)
            })
            .expect("some seal key puts the receipt in the shard group");
        let id = transaction.calculate_id();
        self.executor.script(id, script);
        self.store
            .with_write_tx(|tx| {
                TransactionRecord::new(transaction).insert(tx)?;
                tx.transaction_pool_insert_new(id, Decision::Commit, &Evidence::empty(), true, false, Epoch(100), 0)
            })
            .unwrap();
        id
    }

    /// Moves a pooled transaction to `stage` in a pending update at the parent block.
    fn move_to_stage(&self, id: &TransactionId, stage: TransactionPoolStage) {
        let parent = self.parent.as_leaf();
        self.store
            .with_write_tx(|tx| {
                let mut rec = tx
                    .transaction_pool_get_all(usize::MAX)?
                    .into_iter()
                    .find(|rec| rec.id() == id)
                    .unwrap();
                rec.set_next_stage_and_readiness(stage, parent.shard_group()).unwrap();
                tx.transaction_pool_add_pending_update(&parent, &TransactionPoolStatusUpdate::new(rec, false))
            })
            .unwrap();
    }

    /// Evaluates `atoms` in order as the LocalOnly commands of one block, stopping at the first refusal.
    fn evaluate_local_only(&self, atoms: &[LocalOnlyAtom]) -> Evaluation {
        self.evaluate_local_only_after(atoms, |_| Ok(()))
    }

    /// Evaluates `atoms` as [`Self::evaluate_local_only`] does, in a write transaction in which `before` has already
    /// written.
    fn evaluate_local_only_after<F>(&self, atoms: &[LocalOnlyAtom], before: F) -> Evaluation
    where F: FnOnce(&mut <TestStore as StateStore>::WriteTransaction<'_>) -> Result<(), StorageError> {
        let commands = atoms.iter().cloned().map(Command::LocalOnly).collect::<Vec<_>>();
        self.evaluate_after(&commands, before)
    }

    /// Evaluates `commands` in order as the transaction commands of one block, stopping at the first refusal.
    fn evaluate(&self, commands: &[Command]) -> Evaluation {
        self.evaluate_after(commands, |_| Ok(()))
    }

    /// Evaluates `commands` as [`Self::evaluate`] does, in a write transaction in which `before` has already written.
    /// The commands' executions are speculated first if the replica speculates.
    fn evaluate_after<F>(&self, commands: &[Command], before: F) -> Evaluation
    where F: FnOnce(&mut <TestStore as StateStore>::WriteTransaction<'_>) -> Result<(), StorageError> {
        let block = &self.candidate;
        let mut validator = LocalCommandValidator::new(&self.manager, &self.public_key);
        self.store
            .with_write_tx(|tx| {
                before(tx)?;
                let tx = &**tx;
                let mut substate_store = PendingSubstateStore::init(tx, block.as_leaf(), NUM_PRESHARDS)?;
                let mut change_set = ProposedBlockChangeSet::new(block.as_leaf());
                validator.speculate(tx, block, commands, &self.committee, &substate_store, &change_set);
                let mut evaluation = Evaluation::default();
                for command in commands {
                    evaluation.no_vote = match command {
                        Command::LocalOnly(atom) => validator.evaluate_local_only_command(
                            tx,
                            block,
                            atom,
                            &self.committee,
                            &mut substate_store,
                            &mut change_set,
                            &mut evaluation.total_leader_fee,
                            &mut evaluation.total_exhaust_burn,
                        ),
                        Command::LocalPrepare(atom) => validator.evaluate_local_prepare_command(
                            tx,
                            block,
                            atom,
                            &self.committee,
                            &mut substate_store,
                            &mut change_set,
                        ),
                        Command::LocalAccept(atom) => validator.evaluate_local_accept_command(
                            tx,
                            block,
                            atom,
                            &self.committee,
                            false,
                            &mut substate_store,
                            &mut change_set,
                        ),
                        command => panic!("unsupported command {command}"),
                    }
                    .unwrap();
                    if evaluation.no_vote.is_some() {
                        break;
                    }
                    let id = command.transaction().expect("transaction command").id();
                    let rec = change_set
                        .get_transaction_pool_record(tx, substate_store.parent_chain(), id)
                        .unwrap();
                    evaluation.stages.push(rec.current_stage());
                    evaluation.executed.push(change_set.transaction_execution(id).is_some());
                }
                let mut changes = substate_store
                    .changes()
                    .iter()
                    .map(|change| format!("{change:?}"))
                    .collect::<Vec<_>>();
                changes.sort();
                evaluation.changes = changes;
                evaluation.memo_hits = validator.memo().num_hits();
                evaluation.memo_misses = validator.memo().num_misses();
                Ok::<_, StorageError>(evaluation)
            })
            .unwrap()
    }

    /// Stores pledges of version zero of each of `inputs` from `shard_group` for transaction `id`.
    fn receive_input_pledges(&self, id: &TransactionId, shard_group: ShardGroup, inputs: &[&SubstateId]) {
        let pledges = inputs
            .iter()
            .map(|input| SubstatePledge::Input {
                substate_id: VersionedSubstateId::new((*input).clone(), SubstateVersion::ZERO),
                is_write: !input.is_read_only(),
                substate: Box::new(component_value()),
            })
            .collect();
        self.store
            .with_write_tx(|tx| tx.foreign_substate_pledges_save(id, shard_group, &pledges))
            .unwrap();
    }

    /// The command an honest proposer sends for a New transaction that involves foreign shard groups: LocalAccept if
    /// it aborts or this shard group only creates its outputs, otherwise LocalPrepare.
    fn honest_new_multishard_command(&self, id: &TransactionId) -> Command {
        let block = &self.candidate;
        self.store
            .with_read_tx(|tx| {
                let mut substate_store = PendingSubstateStore::init(tx, block.as_leaf(), NUM_PRESHARDS)?;
                let change_set = ProposedBlockChangeSet::new(block.as_leaf());
                let mut rec = change_set
                    .get_transaction_pool_record(tx, substate_store.parent_chain(), id)
                    .unwrap();
                rec.update_locked_epoch(block.to_locked_epoch());
                let prepared = self
                    .manager
                    .prepare_for_proposal(
                        &mut substate_store,
                        &self.committee,
                        &rec,
                        block.as_leaf(),
                        &change_set,
                        &mut ExecutionMemo::empty(),
                    )
                    .unwrap();
                let PreparedTransaction::MultiShard(multishard) = prepared else {
                    panic!("transaction {id} did not prepare as multi-shard: {prepared:?}");
                };
                let local = self.committee.shard_group();
                match multishard.into_evidence_or_execution() {
                    EvidenceOrExecution::Execution { execution } => {
                        rec.update_from_execution(NUM_PRESHARDS, self.committee.num_committees(), &execution);
                        rec.evidence_mut().add_shard_group(local);
                        if execution.decision().is_commit() && !rec.evidence().has_inputs(local) {
                            let involved = NonZeroU64::new(rec.evidence().num_shard_groups() as u64).unwrap();
                            rec.set_leader_fee(rec.calculate_leader_fee(involved));
                        }
                    },
                    EvidenceOrExecution::Evidence { evidence } => {
                        rec.set_local_decision(Decision::Commit).merge_evidence(evidence);
                    },
                }
                let atom = rec.get_current_multi_shard_atom();
                let command = if rec.current_decision().is_abort() || rec.evidence().is_committee_output_only(local) {
                    Command::LocalAccept(atom)
                } else {
                    Command::LocalPrepare(atom)
                };
                Ok::<_, StorageError>(command)
            })
            .unwrap()
    }
}

/// A block of `shard_group` extending `parent` and carrying `commands`.
fn child(parent: &Block, shard_group: ShardGroup, commands: BlockCommands) -> Block {
    let justify = ProposalCertificate::new(
        *parent.id().hash(),
        *parent.id(),
        parent.height(),
        parent.epoch(),
        ShardGroup::all_shards(NUM_PRESHARDS),
        ProposalCertificate::genesis_signatures(),
        QuorumDecision::Accept,
    );
    let header = BlockHeader::create_unsigned(
        NETWORK,
        ProtocolVersion::V0,
        *parent.id(),
        justify.calculate_id(),
        None,
        parent.height() + NodeHeight(1),
        Epoch::zero(),
        shard_group,
        RistrettoPublicKeyBytes::default(),
        FixedHash::zero(),
        &commands,
        0,
        EpochTime::now().as_u64(),
        FixedHash::zero(),
        ShardGroupAccumulatedData::default(),
        ExtraData::new(),
    )
    .unwrap();
    Block::new(header, justify, commands, None)
}

#[derive(Debug, Default)]
struct Evaluation {
    no_vote: Option<NoVoteReason>,
    total_leader_fee: u64,
    total_exhaust_burn: u128,
    /// The pending stage of each transaction voted for.
    stages: Vec<TransactionPoolStage>,
    /// Whether each transaction voted for has an execution in the block.
    executed: Vec<bool>,
    /// The block's substate changes, in a fixed order.
    changes: Vec<String>,
    memo_hits: usize,
    memo_misses: usize,
}

impl Evaluation {
    /// What the evaluation decided and changed, however it executed the transactions.
    fn outcome(&self) -> impl PartialEq + std::fmt::Debug + '_ {
        (
            format!("{:?}", self.no_vote),
            self.total_leader_fee,
            self.total_exhaust_burn,
            &self.stages,
            &self.executed,
            &self.changes,
        )
    }
}

fn commits() -> Script {
    Script {
        commit: true,
        ..Default::default()
    }
}

fn exhaust_burn() -> u64 {
    FEE / 19
}

/// The leader fee of a transaction that commits in one shard group.
fn local_leader_fee() -> LeaderFee {
    calculate_leader_fee(FEE, exhaust_burn(), NonZeroU64::new(1).unwrap())
}

/// What an honest proposer sends for a transaction that commits.
fn committing(id: TransactionId) -> LocalOnlyAtom {
    LocalOnlyAtom {
        id,
        decision: Decision::Commit,
        transaction_fee: FEE,
        leader_fee: Some(local_leader_fee()),
    }
}

/// What an honest proposer sends for a transaction that aborts with `reason`.
fn aborting(id: TransactionId, reason: AbortReason) -> LocalOnlyAtom {
    LocalOnlyAtom {
        id,
        decision: Decision::Abort(reason),
        transaction_fee: 0,
        leader_fee: None,
    }
}

/// Two committees that each cover half the shards, the first holding the fee resource every transaction reads.
fn local_and_foreign_halves() -> (ShardGroup, ShardGroup) {
    let mut groups = NUM_PRESHARDS.all_shard_groups_iter(2);
    let (lower, upper) = (groups.next().unwrap(), groups.next().unwrap());
    let tari_token = VersionedSubstateId::new(tari_token(), SubstateVersion::ZERO).to_shard(NUM_PRESHARDS);
    if lower.contains(&tari_token) {
        (lower, upper)
    } else {
        (upper, lower)
    }
}

mod local_only {
    use super::*;

    #[test]
    fn an_honest_commit_is_voted_for() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());

        let evaluation = replica.evaluate_local_only(&[committing(id)]);

        assert!(evaluation.no_vote.is_none(), "{:?}", evaluation.no_vote);
        assert_eq!(evaluation.stages, [TransactionPoolStage::LocalOnly]);
        assert_eq!(evaluation.executed, [true]);
        assert_eq!(evaluation.total_leader_fee, local_leader_fee().fee());
        assert_eq!(
            evaluation.total_exhaust_burn,
            u128::from(local_leader_fee().exhaust_burn())
        );
    }

    #[test]
    fn an_honest_abort_is_voted_for() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], Script::default());

        let evaluation = replica.evaluate_local_only(&[aborting(id, AbortReason::ExecutionFailure)]);

        assert!(evaluation.no_vote.is_none(), "{:?}", evaluation.no_vote);
        assert_eq!(evaluation.stages, [TransactionPoolStage::LocalOnly]);
        assert_eq!(evaluation.executed, [true]);
        assert_eq!(evaluation.total_leader_fee, 0);
        assert_eq!(evaluation.total_exhaust_burn, 0);
    }

    #[test]
    fn a_transaction_missing_from_the_pool_is_refused() {
        let replica = Replica::new();

        let evaluation = replica.evaluate_local_only(&[committing(TransactionId::new([9; 32]))]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::TransactionNotInPool)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_transaction_past_the_new_stage_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());
        replica.move_to_stage(&id, TransactionPoolStage::LocalOnly);

        let evaluation = replica.evaluate_local_only(&[committing(id)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::StageDisagreement {
                    stage: TransactionPoolStage::LocalOnly,
                    expected: TransactionPoolStage::New,
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_abort_proposed_for_a_commit_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());

        let evaluation = replica.evaluate_local_only(&[aborting(id, AbortReason::ExecutionFailure)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    local: Decision::Commit,
                    remote: Decision::Abort(AbortReason::ExecutionFailure),
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_commit_proposed_for_an_execution_abort_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], Script::default());

        let evaluation = replica.evaluate_local_only(&[committing(id)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    remote: Decision::Commit,
                    ..
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_commit_proposed_for_a_missing_input_is_refused() {
        let replica = Replica::new();
        let id = replica.pool_transaction(1, &[&component_id(0x20)], commits());

        let evaluation = replica.evaluate_local_only(&[committing(id)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    remote: Decision::Commit,
                    ..
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_different_transaction_fee_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());

        let evaluation = replica.evaluate_local_only(&[LocalOnlyAtom {
            transaction_fee: FEE + 1,
            ..committing(id)
        }]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::FeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_commit_without_a_leader_fee_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());

        let evaluation = replica.evaluate_local_only(&[LocalOnlyAtom {
            leader_fee: None,
            ..committing(id)
        }]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::NoLeaderFee)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_different_leader_fee_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());

        let evaluation = replica.evaluate_local_only(&[LocalOnlyAtom {
            leader_fee: Some(calculate_leader_fee(FEE, exhaust_burn(), NonZeroU64::new(2).unwrap())),
            ..committing(id)
        }]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LeaderFeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_transaction_whose_diff_spends_what_an_earlier_one_in_the_block_spent_is_refused() {
        let replica = Replica::new();
        let spent = component_id(0x20);
        let other = component_id(0x21);
        replica.add_substate(&spent);
        replica.add_substate(&other);
        let first = replica.pool_transaction(1, &[&spent], commits());
        let second = replica.pool_transaction(2, &[&other], Script {
            commit: true,
            extra_downs: vec![VersionedSubstateId::new(spent, SubstateVersion::ZERO)],
            ..Default::default()
        });

        let evaluation = replica.evaluate_local_only(&[committing(first), committing(second)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::ConflictingTransactionProposed { transaction_id, .. }) if transaction_id == second
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_transaction_with_a_foreign_output_is_refused() {
        let (local, foreign) = local_and_foreign_halves();
        let replica = Replica::in_committee(local, 2);
        let input = component_id_in(local);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], Script {
            commit: true,
            new_outputs: vec![component_id_in(foreign)],
            ..Default::default()
        });

        let evaluation = replica.evaluate_local_only(&[committing(id)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LocalOnlyProposedForMultiShard)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_local_only_execution_with_evidence_in_a_foreign_shard_group_is_refused() {
        let (local, foreign) = local_and_foreign_halves();
        let replica = Replica::in_committee(local, 2);
        let input = component_id_in(local);
        let foreign_read = component_id_in(foreign);
        replica.add_substate(&input);
        // Locking a read needs the substate in this replica's store
        replica.add_substate(&foreign_read);
        let id = replica.pool_transaction(1, &[&input], Script {
            commit: true,
            extra_reads: vec![foreign_read],
            ..Default::default()
        });

        let evaluation = replica.evaluate_local_only(&[committing(id)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LocalOnlyProposedForMultiShard)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_honest_early_abort_is_voted_for() {
        let replica = Replica::new();
        let id = replica.pool_transaction(1, &[&component_id(0x20)], commits());

        let evaluation = replica.evaluate_local_only(&[aborting(id, AbortReason::OneOrMoreInputsNotFound)]);

        assert!(evaluation.no_vote.is_none(), "{:?}", evaluation.no_vote);
        assert_eq!(evaluation.stages, [TransactionPoolStage::LocalOnly]);
    }

    #[test]
    fn an_early_abort_proposed_for_a_different_reason_is_refused() {
        let replica = Replica::new();
        let id = replica.pool_transaction(1, &[&component_id(0x20)], commits());

        let evaluation = replica.evaluate_local_only(&[aborting(id, AbortReason::ExecutionFailure)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    local: Decision::Abort(AbortReason::OneOrMoreInputsNotFound),
                    remote: Decision::Abort(AbortReason::ExecutionFailure),
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_early_abort_with_a_transaction_fee_is_refused() {
        let replica = Replica::new();
        let id = replica.pool_transaction(1, &[&component_id(0x20)], commits());

        let evaluation = replica.evaluate_local_only(&[LocalOnlyAtom {
            transaction_fee: FEE,
            ..aborting(id, AbortReason::OneOrMoreInputsNotFound)
        }]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::FeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_abort_with_a_leader_fee_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], Script::default());

        let evaluation = replica.evaluate_local_only(&[LocalOnlyAtom {
            leader_fee: Some(local_leader_fee()),
            ..aborting(id, AbortReason::ExecutionFailure)
        }]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LeaderFeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }
}

/// The atom of a LocalPrepare or LocalAccept command.
fn multishard_atom(command: &Command) -> MultiShardAtom {
    match command {
        Command::LocalPrepare(atom) | Command::LocalAccept(atom) => atom.clone(),
        command => panic!("{command} is not a multi-shard command"),
    }
}

/// A replica of the committee holding the fee resource, with a pooled transaction that writes a local component and
/// creates one in the other committee. It executes here, so it prepares with an execution.
fn executing_cross_shard() -> (Replica, TransactionId) {
    let (local, foreign) = local_and_foreign_halves();
    let replica = Replica::in_committee(local, 2);
    let input = component_id_in(local);
    replica.add_substate(&input);
    let id = replica.pool_transaction(1, &[&input], Script {
        commit: true,
        new_outputs: vec![component_id_in(foreign)],
        ..Default::default()
    });
    (replica, id)
}

/// A replica of the committee holding the fee resource, with a pooled transaction that also writes a component in the
/// other committee. It cannot execute here, so it prepares by pledging its local inputs.
fn pledging_cross_shard() -> (Replica, TransactionId) {
    let (local, foreign) = local_and_foreign_halves();
    let replica = Replica::in_committee(local, 2);
    let input = component_id_in(local);
    replica.add_substate(&input);
    let id = replica.pool_transaction(1, &[&input, &component_id_in(foreign)], commits());
    (replica, id)
}

/// A replica of the committee holding the fee resource, with a pooled transaction whose local input does not exist
/// and that also writes a component in the other committee. It aborts while preparing.
fn aborting_cross_shard() -> (Replica, TransactionId) {
    let (local, foreign) = local_and_foreign_halves();
    let replica = Replica::in_committee(local, 2);
    let id = replica.pool_transaction(1, &[&component_id_in(local), &component_id_in(foreign)], commits());
    (replica, id)
}

/// A replica of the committee without the fee resource, with a pooled transaction whose inputs are all in the other
/// committee and pledged from it. This committee only creates its receipt.
fn output_only() -> (Replica, TransactionId) {
    let (input_group, local) = local_and_foreign_halves();
    let replica = Replica::in_committee(local, 2);
    let input = component_id_in(input_group);
    let id = replica.pool_transaction(1, &[&input], commits());
    replica.receive_input_pledges(&id, input_group, &[&input, &tari_token()]);
    (replica, id)
}

fn assert_voted(evaluation: &Evaluation, stage: TransactionPoolStage, executed: bool) {
    assert!(evaluation.no_vote.is_none(), "{:?}", evaluation.no_vote);
    assert_eq!(evaluation.stages, [stage]);
    assert_eq!(evaluation.executed, [executed]);
}

mod local_prepare {
    use super::*;

    #[test]
    fn an_honest_executed_prepare_is_voted_for() {
        let (replica, id) = executing_cross_shard();
        let command = replica.honest_new_multishard_command(&id);
        assert!(matches!(command, Command::LocalPrepare(_)), "{command}");

        let evaluation = replica.evaluate(&[command]);

        assert_voted(&evaluation, TransactionPoolStage::LocalPrepared, true);
    }

    #[test]
    fn an_honest_pledging_prepare_is_voted_for() {
        let (replica, id) = pledging_cross_shard();
        let command = replica.honest_new_multishard_command(&id);
        assert!(matches!(command, Command::LocalPrepare(_)), "{command}");

        let evaluation = replica.evaluate(&[command]);

        assert_voted(&evaluation, TransactionPoolStage::LocalPrepared, false);
    }

    #[test]
    fn a_transaction_missing_from_the_pool_is_refused() {
        let (replica, id) = executing_cross_shard();
        let atom = MultiShardAtom {
            id: TransactionId::new([9; 32]),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::TransactionNotInPool)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_transaction_past_the_new_stage_is_refused() {
        let (replica, id) = executing_cross_shard();
        let command = replica.honest_new_multishard_command(&id);
        replica.move_to_stage(&id, TransactionPoolStage::LocalPrepared);

        let evaluation = replica.evaluate(&[command]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::StageDisagreement {
                    stage: TransactionPoolStage::LocalPrepared,
                    expected: TransactionPoolStage::New,
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_local_only_transaction_is_refused() {
        let replica = Replica::new();
        let input = component_id(0x20);
        replica.add_substate(&input);
        let id = replica.pool_transaction(1, &[&input], commits());

        let evaluation = replica.evaluate(&[Command::LocalPrepare(MultiShardAtom {
            id,
            decision: Decision::Commit,
            evidence: Evidence::empty(),
            transaction_fee: FEE,
            leader_fee: None,
        })]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::MultiShardProposedForLocalOnly)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_abort_proposed_for_a_commit_is_refused() {
        let (replica, id) = executing_cross_shard();
        let atom = MultiShardAtom {
            decision: Decision::Abort(AbortReason::ExecutionFailure),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    local: Decision::Commit,
                    remote: Decision::Abort(AbortReason::ExecutionFailure),
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_different_transaction_fee_is_refused() {
        let (replica, id) = executing_cross_shard();
        let honest = multishard_atom(&replica.honest_new_multishard_command(&id));
        let atom = MultiShardAtom {
            transaction_fee: honest.transaction_fee + 1,
            ..honest
        };

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::FeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn different_local_evidence_is_refused() {
        let (replica, id) = pledging_cross_shard();
        let atom = MultiShardAtom {
            evidence: Evidence::empty(),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::InvalidEvidence {
                    reason: InvalidEvidenceReason::MismatchedEvidence
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }
}

mod local_accept_from_new {
    use super::*;

    #[test]
    fn an_honest_abort_is_voted_for() {
        let (replica, id) = aborting_cross_shard();
        let command = replica.honest_new_multishard_command(&id);
        assert!(
            matches!(&command, Command::LocalAccept(atom) if atom.decision.is_abort()),
            "{command}"
        );

        let evaluation = replica.evaluate(&[command]);

        assert_voted(&evaluation, TransactionPoolStage::LocalAccepted, true);
    }

    #[test]
    fn an_honest_output_only_commit_is_voted_for() {
        let (replica, id) = output_only();
        let command = replica.honest_new_multishard_command(&id);
        assert!(
            matches!(&command, Command::LocalAccept(atom) if atom.decision.is_commit() && atom.leader_fee.is_some()),
            "{command}"
        );

        let evaluation = replica.evaluate(&[command]);

        assert_voted(&evaluation, TransactionPoolStage::LocalAccepted, true);
    }

    #[test]
    fn a_transaction_that_prepares_is_refused() {
        let (replica, id) = executing_cross_shard();
        let atom = multishard_atom(&replica.honest_new_multishard_command(&id));

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::CommandDisagreement {
                    local: "LocalPrepare",
                    remote: "LocalAccept",
                    ..
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_commit_proposed_for_an_abort_is_refused() {
        let (replica, id) = aborting_cross_shard();
        let atom = MultiShardAtom {
            decision: Decision::Commit,
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    remote: Decision::Commit,
                    ..
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_different_transaction_fee_is_refused() {
        let (replica, id) = output_only();
        let honest = multishard_atom(&replica.honest_new_multishard_command(&id));
        let atom = MultiShardAtom {
            transaction_fee: honest.transaction_fee + 1,
            ..honest
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::FeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_commit_without_a_leader_fee_is_refused() {
        let (replica, id) = output_only();
        let atom = MultiShardAtom {
            leader_fee: None,
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::NoLeaderFee)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_different_leader_fee_is_refused() {
        let (replica, id) = output_only();
        let atom = MultiShardAtom {
            leader_fee: Some(local_leader_fee()),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LeaderFeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_abort_with_a_leader_fee_is_refused() {
        let (replica, id) = aborting_cross_shard();
        let atom = MultiShardAtom {
            leader_fee: Some(local_leader_fee()),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LeaderFeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn different_evidence_is_refused() {
        let (replica, id) = output_only();
        let atom = MultiShardAtom {
            evidence: Evidence::empty(),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::InvalidEvidence {
                    reason: InvalidEvidenceReason::MismatchedEvidence
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }
}

/// Rules a replica enforces by comparing a proposed command for a New cross-shard transaction with the one it derives.
mod new_multishard_rules {
    use super::*;

    fn assert_command_disagreement(evaluation: &Evaluation, local: &str, remote: &str) {
        assert!(
            matches!(
                &evaluation.no_vote,
                Some(NoVoteReason::CommandDisagreement { local: l, remote: r, .. }) if *l == local && *r == remote
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_prepare_for_an_abort_is_refused() {
        let (replica, id) = aborting_cross_shard();
        let atom = multishard_atom(&replica.honest_new_multishard_command(&id));

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert_command_disagreement(&evaluation, "LocalAccept", "LocalPrepare");
    }

    #[test]
    fn a_prepare_for_an_output_only_transaction_is_refused() {
        let (replica, id) = output_only();
        let atom = multishard_atom(&replica.honest_new_multishard_command(&id));

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert_command_disagreement(&evaluation, "LocalAccept", "LocalPrepare");
    }

    #[test]
    fn a_prepare_with_a_leader_fee_is_refused() {
        let (replica, id) = executing_cross_shard();
        let atom = MultiShardAtom {
            leader_fee: Some(local_leader_fee()),
            ..multishard_atom(&replica.honest_new_multishard_command(&id))
        };

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert!(
            matches!(evaluation.no_vote, Some(NoVoteReason::LeaderFeeDisagreement)),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn a_prepare_with_different_foreign_evidence_is_refused() {
        let (replica, id) = pledging_cross_shard();
        let mut atom = multishard_atom(&replica.honest_new_multishard_command(&id));
        atom.evidence.add_shard_group(ShardGroup::all_shards(NUM_PRESHARDS));

        let evaluation = replica.evaluate(&[Command::LocalPrepare(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::InvalidEvidence {
                    reason: InvalidEvidenceReason::MismatchedEvidence
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_abort_for_a_different_reason_is_refused() {
        let (replica, id) = aborting_cross_shard();
        let honest = multishard_atom(&replica.honest_new_multishard_command(&id));
        assert_eq!(honest.decision, Decision::Abort(AbortReason::OneOrMoreInputsNotFound));
        let atom = MultiShardAtom {
            decision: Decision::Abort(AbortReason::ExecutionFailure),
            ..honest
        };

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::DecisionDisagreement {
                    local: Decision::Abort(AbortReason::OneOrMoreInputsNotFound),
                    remote: Decision::Abort(AbortReason::ExecutionFailure),
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }

    #[test]
    fn an_accept_with_a_certificate_in_its_evidence_is_refused() {
        let (replica, id) = output_only();
        let mut atom = multishard_atom(&replica.honest_new_multishard_command(&id));
        let (input_group, _) = local_and_foreign_halves();
        atom.evidence.add_shard_group(input_group).set_prepare_qc(PcId::zero());

        let evaluation = replica.evaluate(&[Command::LocalAccept(atom)]);

        assert!(
            matches!(
                evaluation.no_vote,
                Some(NoVoteReason::InvalidEvidence {
                    reason: InvalidEvidenceReason::MismatchedEvidence
                })
            ),
            "{:?}",
            evaluation.no_vote
        );
    }
}

mod speculation {
    use super::*;

    const THREADS: usize = 4;

    /// Builds the same pool in a replica that evaluates without speculating and one that speculates on [THREADS]
    /// threads, with `setup`, and evaluates the atoms it returns, sorted into block order, in each.
    fn evaluate_both(setup: impl Fn(&Replica) -> Vec<LocalOnlyAtom>) -> (Evaluation, Evaluation, Replica) {
        let serial = Replica::new();
        let speculating = Replica::new().with_execution_threads(THREADS);
        let mut atoms = setup(&serial);
        assert_eq!(setup(&speculating), atoms, "both replicas pool the same transactions");
        atoms.sort_by_key(|atom| atom.id);
        let expected = serial.evaluate_local_only(&atoms);
        let speculated = speculating.evaluate_local_only(&atoms);
        (expected, speculated, speculating)
    }

    #[test]
    fn disjoint_transactions_take_their_speculated_executions() {
        let (expected, speculated, replica) = evaluate_both(|replica| {
            (0..4u8)
                .map(|n| {
                    let input = component_id(0x20 + n);
                    replica.add_substate(&input);
                    committing(replica.pool_transaction(u64::from(n) + 1, &[&input], commits()))
                })
                .collect()
        });

        assert!(expected.no_vote.is_none(), "{:?}", expected.no_vote);
        assert_eq!(speculated.outcome(), expected.outcome());
        assert_eq!(speculated.memo_hits, 4);
        assert_eq!(speculated.memo_misses, 0);
        for id in replica.executor.num_executions.lock().unwrap().values() {
            assert_eq!(*id, 1, "a speculated transaction is executed once");
        }
    }

    #[test]
    fn a_chain_through_one_substate_takes_every_speculated_execution() {
        let (expected, speculated, _replica) = evaluate_both(|replica| {
            let vault = component_id(0x20);
            replica.add_substate(&vault);
            (1..=4)
                .map(|seed| committing(replica.pool_transaction(seed, &[&vault], commits())))
                .collect()
        });

        assert!(expected.no_vote.is_none(), "{:?}", expected.no_vote);
        assert_eq!(speculated.outcome(), expected.outcome());
        assert_eq!(speculated.memo_hits, 4);
    }

    #[test]
    fn a_transaction_using_a_substate_an_earlier_one_creates_is_executed_in_block_order() {
        let created = component_id(0x40);
        let (expected, speculated, replica) = evaluate_both(|replica| {
            let input = component_id(0x20);
            replica.add_substate(&input);
            let creator = replica.pool_transaction(1, &[&input], Script {
                commit: true,
                new_outputs: vec![created.clone()],
                ..Default::default()
            });
            // The user declares only the created substate, so it shares no unit with the creator
            let user = (2..)
                .map(|seed| replica.pool_transaction(seed, &[&created], commits()))
                .find(|user| *user > creator)
                .unwrap();
            vec![committing(creator), committing(user)]
        });

        assert!(expected.no_vote.is_none(), "{:?}", expected.no_vote);
        assert_eq!(speculated.outcome(), expected.outcome());
        assert_eq!(speculated.memo_hits, 1, "only the creator is speculated");
        let executions = replica.executor.num_executions.lock().unwrap().clone();
        assert!(executions.values().all(|n| *n == 1), "{executions:?}");
    }

    #[test]
    fn a_snapshot_behind_the_write_transaction_misses_and_the_block_is_unchanged() {
        let input = component_id(0x20);
        let setup = |replica: &Replica| {
            replica.add_substate(&input);
            vec![committing(replica.pool_transaction(1, &[&input], commits()))]
        };
        // The evaluation's write transaction moves the input to version 1 before the block is evaluated, as a
        // replica's commit of the block's ancestors does. The snapshot speculation reads cannot see it.
        let bump_input = |tx: &mut <TestStore as StateStore>::WriteTransaction<'_>| {
            let shard = VersionedSubstateId::new(input.clone(), SubstateVersion::ZERO).to_shard(NUM_PRESHARDS);
            let mut batch = SubstateUpdateBatch::new(NETWORK, Epoch::zero());
            let transitions = batch.with_transition(shard, 1);
            transitions.push(SubstateTransition::Down {
                id: VersionedSubstateId::new(input.clone(), SubstateVersion::ZERO),
            });
            transitions.push(SubstateTransition::Up {
                id: input.clone(),
                version: SubstateVersion::new(1),
                substate_or_hash: component_value().into(),
            });
            SubstateRecord::commit_batch(tx, batch)
        };

        let serial = Replica::new();
        let atoms = setup(&serial);
        let expected = serial.evaluate_local_only_after(&atoms, bump_input);
        let speculating = Replica::new().with_execution_threads(THREADS);
        assert_eq!(setup(&speculating), atoms);
        let speculated = speculating.evaluate_local_only_after(&atoms, bump_input);

        assert!(expected.no_vote.is_none(), "{:?}", expected.no_vote);
        assert_eq!(speculated.outcome(), expected.outcome());
        assert_eq!(speculated.memo_hits, 0);
        assert_eq!(speculated.memo_misses, 1);
        assert_eq!(speculating.executor.num_executions(atoms[0].id()), 2);
    }

    /// A deterministic source of small random numbers.
    struct XorShift(u64);

    impl XorShift {
        fn below(&mut self, bound: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % bound
        }
    }

    /// Pools transactions that write random substates among a few, one of them written by many as a faucet's vault
    /// is. Some use substates created by an earlier transaction in the block without that transaction declaring them.
    fn random_block(replica: &Replica, seed: u64) -> Vec<LocalOnlyAtom> {
        const NUM_SUBSTATES: u8 = 5;
        const NUM_TRANSACTIONS: u64 = 10;
        const NUM_CREATED: u8 = 3;
        let mut rng = XorShift(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        let existing = (0..NUM_SUBSTATES).map(|n| component_id(0x20 + n)).collect::<Vec<_>>();
        for id in &existing {
            replica.add_substate(id);
        }
        let created = (0..NUM_CREATED).map(|n| component_id(0x40 + n)).collect::<Vec<_>>();

        let mut transactions = (0..NUM_TRANSACTIONS)
            .map(|n| {
                let mut inputs = Vec::new();
                if rng.below(2) == 0 {
                    // The faucet's vault
                    inputs.push(&existing[0]);
                }
                let other = &existing[rng.below(u64::from(NUM_SUBSTATES)) as usize];
                if !inputs.contains(&other) {
                    inputs.push(other);
                }
                if rng.below(4) == 0 {
                    inputs.push(&created[rng.below(u64::from(NUM_CREATED)) as usize]);
                }
                let id = replica.pool_transaction(seed * 100 + n + 1, &inputs, commits());
                (id, inputs.into_iter().cloned().collect::<Vec<_>>())
            })
            .collect::<Vec<_>>();
        transactions.sort_by_key(|(id, _)| *id);

        // Each created substate is created by a transaction ordered before its first user, or exists already if its
        // first user is the first transaction
        for substate in &created {
            let Some(first_user) = transactions.iter().position(|(_, inputs)| inputs.contains(substate)) else {
                continue;
            };
            if first_user == 0 {
                replica.add_substate(substate);
                continue;
            }
            let (creator, _) = &transactions[rng.below(first_user as u64) as usize];
            let mut scripts = replica.executor.scripts.lock().unwrap();
            scripts.get_mut(creator).unwrap().new_outputs.push(substate.clone());
        }

        transactions.into_iter().map(|(id, _)| committing(id)).collect()
    }

    #[test]
    fn random_conflict_graphs_evaluate_the_same_with_and_without_speculation() {
        let (mut total_hits, mut total_misses) = (0, 0);
        for seed in 1..=24 {
            let (expected, speculated, _replica) = evaluate_both(|replica| random_block(replica, seed));
            assert_eq!(speculated.outcome(), expected.outcome(), "seed {seed}");
            total_hits += speculated.memo_hits;
            total_misses += speculated.memo_misses;
        }
        assert!(total_hits > 0, "no speculated execution was taken");
        assert!(total_misses > 0, "no speculation read state the block did not have");
    }
}
