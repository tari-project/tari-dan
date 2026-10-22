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
use tari_consensus_types::{Decision, ProposalCertificate, ShardGroupAccumulatedData};
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
        Evidence,
        LeaderFee,
        LocalOnlyAtom,
        LockedEpoch,
        NoVoteReason,
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
        transaction_manager::ConsensusTransactionManager,
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
}

impl ScriptedExecutor {
    fn script(&self, transaction_id: TransactionId, script: Script) {
        self.scripts.lock().unwrap().insert(transaction_id, script);
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

/// A chain of the zero block and an uncommitted child of it, the parent of the block a replica evaluates.
struct Replica {
    store: TestStore,
    executor: ScriptedExecutor,
    manager: ConsensusTransactionManager<ScriptedExecutor, TestStore>,
    committee: CommitteeInfo,
    parent: Block,
    public_key: RistrettoPublicKey,
    _tmp: TempDir,
}

impl Replica {
    /// A replica whose committee is the only one in the network.
    fn new() -> Self {
        Self::in_committee(ShardGroup::all_shards(NUM_PRESHARDS), 1)
    }

    /// A replica whose committee covers `shard_group` of a network split into `num_committees`.
    fn in_committee(shard_group: ShardGroup, num_committees: u32) -> Self {
        let tmp = tempfile::tempdir().unwrap();
        let store = TestStore::open(tmp.path(), DatabaseOptions::default()).unwrap();
        let zero = Block::zero_block(NETWORK, NUM_PRESHARDS);
        let parent = child(&zero, shard_group, BlockCommands::empty());
        store
            .with_write_tx(|tx| {
                for block in [&zero, &parent] {
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

    /// A block extending the parent and carrying `commands`.
    fn block(&self, commands: BlockCommands) -> Block {
        child(&self.parent, self.committee.shard_group(), commands)
    }

    /// Evaluates `atoms` in order as the LocalOnly commands of one block, stopping at the first refusal.
    fn evaluate_local_only(&self, atoms: &[LocalOnlyAtom]) -> Evaluation {
        let block = self.block(BlockCommands::empty());
        self.store
            .with_write_tx(|tx| {
                block.justify().save(tx)?;
                block.insert(tx)
            })
            .unwrap();
        let validator = LocalCommandValidator::new(&self.manager, &self.public_key);
        self.store
            .with_read_tx(|tx| {
                let mut substate_store = PendingSubstateStore::init(tx, block.as_leaf(), NUM_PRESHARDS)?;
                let mut change_set = ProposedBlockChangeSet::new(block.as_leaf());
                let mut evaluation = Evaluation::default();
                for atom in atoms {
                    evaluation.no_vote = validator
                        .evaluate_local_only_command(
                            tx,
                            &block,
                            atom,
                            &self.committee,
                            &mut substate_store,
                            &mut change_set,
                            &mut evaluation.total_leader_fee,
                            &mut evaluation.total_exhaust_burn,
                        )
                        .unwrap();
                    if evaluation.no_vote.is_some() {
                        break;
                    }
                    let rec = change_set
                        .get_transaction_pool_record(tx, substate_store.parent_chain(), atom.id())
                        .unwrap();
                    evaluation.stages.push(rec.current_stage());
                    evaluation
                        .executed
                        .push(change_set.transaction_execution(atom.id()).is_some());
                }
                Ok::<_, StorageError>(evaluation)
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
}
