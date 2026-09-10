//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use log::*;
use ootle_network::Network;
use tari_engine_types::{
    commit_result::{FinalizeResult, RejectReason, TransactionResult},
    component::{Component, ComponentBody, ComponentHeader},
    events::Event,
    fees::{ExhaustBurnRate, FeeBreakdown, FeeReceipt, FeeSource},
    indexed_value::{IndexedValue, IndexedWellKnownTypes},
    limits,
    lock::LockFlag,
    logs::LogEntry,
    substate::{Substate, SubstateId, SubstateValue},
    transaction_receipt::FinalizeOutcome,
    virtual_substate::VirtualSubstates,
};
use tari_ootle_common_types::Epoch;
use tari_ootle_transaction::TransactionWeight;
use tari_template_lib::{
    models::ComponentAddressAllocation,
    types::{
        ComponentAddress,
        Hash32,
        Metadata,
        SubstateOwnerRule,
        TemplateAddress,
        access_rules::ComponentAccessRules,
    },
};

use crate::{
    fees::WasmMeteringRate,
    runtime::{
        RuntimeError,
        error::ArgumentValidationError,
        locking::LockedSubstate,
        scope::{CallScope, FrameWriteMode, PushCallFrame},
        working_state::{ChargeableState, WorkingState},
        workspace::Workspace,
    },
    state_store::StateReader,
};

const LOG_TARGET: &str = "tari::ootle::engine::runtime::state_tracker";

/// The state finalization will persist, detached from the tracker and awaiting fee settlement.
#[derive(Debug)]
pub struct FinalizedState<TStore> {
    state: WorkingState<TStore>,
    outcome: FinalizeOutcome,
    /// Why the main intent was rejected, when this is a fee-intent commit.
    reason: Option<RejectReason>,
    /// What committing the whole transaction was priced at, captured before the second charging
    /// pass re-derives the charges over whichever state was chosen.
    total_fees_required: u64,
}

impl<TStore> FinalizedState<TStore> {
    pub fn chargeable_state(&mut self) -> ChargeableState<'_, TStore> {
        ChargeableState::new(&mut self.state)
    }

    pub fn outcome(&self) -> FinalizeOutcome {
        self.outcome
    }
}

/// The compute a transaction may still consume, and what authorizes it.
#[derive(Debug, Clone, Copy)]
pub struct ComputeAllowance {
    /// Metering points, counting WASM execution and native verification together.
    pub points: u64,
    pub funding: ComputeFunding,
}

/// What is paying for the compute an allowance authorizes. Which one binds decides what a
/// transaction that exceeds it is told: a payment-funded allowance rises with the fee, the fee
/// intent's credit does not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComputeFunding {
    /// The flat credit the fee intent runs on, sized to source a fee and nothing more.
    FeeIntentCredit,
    /// What the payment has left after the charges standing against it.
    Payment,
}

#[derive(Debug)]
pub struct StateTracker<TStore> {
    working_state: Option<WorkingState<TStore>>,
    fee_checkpoint: Option<WorkingState<TStore>>,
    transaction_weight: TransactionWeight,
    wasm_metering_rate: WasmMeteringRate,
    /// Stealth transfers performed so far in the fee intent. Lives here rather than in `WorkingState` because the fee
    /// intent is delimited by `fee_checkpoint`, and because the checkpoint clones the working state — a count held
    /// there would be duplicated into the clone.
    fee_intent_stealth_transfers: usize,
}

impl<TStore: StateReader> StateTracker<TStore> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_store: TStore,
        virtual_substates: VirtualSubstates,
        initial_call_scope: CallScope,
        transaction_hash: Hash32,
        intent_commitment: Hash32,
        transaction_weight: TransactionWeight,
        wasm_metering_rate: WasmMeteringRate,
        burn_rate: ExhaustBurnRate,
        network: Network,
        dry_run: bool,
    ) -> Self {
        Self {
            working_state: Some(WorkingState::new(
                state_store,
                virtual_substates,
                initial_call_scope,
                transaction_hash,
                intent_commitment,
                burn_rate,
                network,
                dry_run,
            )),
            fee_checkpoint: None,
            transaction_weight,
            wasm_metering_rate,
            fee_intent_stealth_transfers: 0,
        }
    }

    pub fn get_transaction_weight(&self) -> TransactionWeight {
        self.transaction_weight
    }

    /// The compute this transaction may still consume, and what authorizes it. `None` when no bound
    /// applies beyond the per-transaction hard cap — WASM execution is not priced, or this is a dry
    /// run. Used by `WasmProcess::invoke` to cap each call's metering allowance, and by native
    /// verification to pre-charge against the same figure.
    ///
    /// The fee intent runs on a flat [`limits::FREE_COMPUTE_GRACE_POINTS`] of credit, whatever it
    /// has paid. Sourcing a fee is the whole reason a transaction may run anything before paying,
    /// and that is all the credit is sized for. Letting a payment raise this allowance would make
    /// the fee intent the only place worth executing anything: a failure there leaves no checkpoint
    /// to fall back to, so the transaction settles as a rejection that collects nothing, and the
    /// work would be done and never paid for — repeatably, with the same funds. Work that needs
    /// more than the credit belongs in the main intent, where the fee already paid funds it and a
    /// failure still commits the fee intent.
    ///
    /// Past the checkpoint the credit ends and compute is funded by what the payment has *left*,
    /// not by the whole of it. The charges standing at the checkpoint are what the fallback commit
    /// costs, so funding compute out of the full payment would let a transaction spend the lot on
    /// execution and leave its own fee-intent commit unaffordable — a rejection that again collects
    /// nothing.
    ///
    /// That payment-funded figure is measured against the charges *standing when it is asked*.
    /// Anything charged after the last call — a host call inside the final invocation — is outside
    /// it, so it bounds the unpaid work rather than reducing it to zero.
    pub fn compute_allowance(&self) -> Option<ComputeAllowance> {
        let rate = self.wasm_metering_rate;
        let is_fee_intent = self.fee_checkpoint.is_none();
        self.read_with(|state| {
            if !rate.prices_execution() {
                return None;
            }
            let fee_state = state.fee_state();
            if is_fee_intent {
                // The credit binds a dry run as it binds a real one. It is the same figure either
                // way, so estimating against it costs no accuracy and is where a wallet finds out
                // that the work has to move to the main instructions.
                return Some(ComputeAllowance {
                    points: limits::FREE_COMPUTE_GRACE_POINTS,
                    funding: ComputeFunding::FeeIntentCredit,
                });
            }
            // A dry run is metered at whatever `max_fee` the caller submitted, so past the
            // checkpoint there is no payment to derive a bound from.
            if fee_state.is_dry_run() {
                return None;
            }
            let unspent = fee_state.total_payments().saturating_sub(fee_state.total_charges());
            Some(ComputeAllowance {
                // `prices_execution` above is the only case that yields no figure, so nothing here
                // may widen the allowance by failing to produce one.
                points: rate.points_funded_by(unspent).unwrap_or(0),
                funding: ComputeFunding::Payment,
            })
        })
    }

    pub fn get_current_epoch(&self) -> Result<Epoch, RuntimeError> {
        self.read_with(|state| state.get_current_epoch())
    }

    pub fn get_current_epoch_hash(&self) -> Result<Hash32, RuntimeError> {
        self.read_with(|state| state.get_current_epoch_hash())
    }

    pub fn get_pseudorandom_bytes(&self, length: usize) -> Result<Vec<u8>, RuntimeError> {
        self.read_with(|state| {
            let id_provider = state.id_provider()?;
            // TODO: epoch_hash is a bad source of entropy. Agreeing on randomness at a consensus level is challenging
            // in multi-sharded consensus.
            let epoch_hash = state.get_current_epoch_hash()?;
            let bytes = id_provider.get_random_bytes(&epoch_hash, length)?;
            Ok(bytes)
        })
    }

    pub fn add_event(&mut self, event: Event) -> Result<(), RuntimeError> {
        debug!(target: LOG_TARGET, "Emit: {event}");
        self.write_with(|state| state.push_event(event))
    }

    pub fn add_log(&mut self, log: LogEntry) -> Result<(), RuntimeError> {
        self.write_with(|state| state.push_log(log))
    }

    /// Returns `true` the first time a given template address is seen within this transaction's
    /// state, `false` thereafter. Callers use this to dedupe `FeeSource::TemplateLoad` charges:
    /// the validator pays the cold compile/deserialise cost at most once per template per
    /// process, so charging it on every entry over-bills the user.
    pub fn record_template_load_charge(&mut self, address: TemplateAddress) -> bool {
        self.write_with(|state| state.record_template_load_charge(address))
    }

    pub fn take_events(&mut self) -> Vec<Event> {
        self.write_with(|state| state.take_events())
    }

    pub fn get_template_address(&self) -> Result<TemplateAddress, RuntimeError> {
        self.read_with(|state| state.current_template().copied())
    }

    pub fn get_template_module_name(&self) -> Result<String, RuntimeError> {
        self.read_with(|state| state.current_template_name().map(ToOwned::to_owned))
    }

    pub fn new_component(
        &mut self,
        component_state: tari_bor::Value,
        owner_rule: SubstateOwnerRule,
        access_rules: ComponentAccessRules,
        address_allocation: Option<ComponentAddressAllocation>,
    ) -> Result<ComponentAddress, RuntimeError> {
        self.write_with(|state| {
            let template_address = *state.current_template()?;

            let component_address = match address_allocation {
                Some(address_allocation) => {
                    let alloc = state.use_allocated_address(address_allocation.id())?;
                    alloc.substate_id().as_component_address().ok_or_else(|| {
                        RuntimeError::AddressAllocationTypeMismatch {
                            id: alloc.substate_id().clone(),
                            expected: "ComponentAddress",
                        }
                    })?
                },
                None => state.id_provider()?.new_component_address()?,
            };

            let body = ComponentBody { state: component_state };
            let header = ComponentHeader {
                template_address,
                access_rules,
                owner_rule,
                entity_id: component_address.entity_id(),
            };

            let component = Component { header, body };
            let substate_id = SubstateId::Component(component_address);
            // The template address/component_id combination will not necessarily be unique so we need to check this.
            if state.substate_exists(&substate_id)? {
                return Err(RuntimeError::ComponentAlreadyExists {
                    address: component_address,
                });
            }

            let indexed = IndexedWellKnownTypes::from_value(&component.body.state)?;
            state.validate_component_state(None, &indexed)?;

            state.new_substate(substate_id.clone(), SubstateValue::Component(component))?;

            state.push_event(Event::std(
                Some(substate_id),
                template_address,
                "component",
                "created",
                Metadata::new(),
            ))?;

            debug!(target: LOG_TARGET, "New component created: {}", component_address);
            Ok(component_address)
        })
    }

    pub fn lock_substate(&mut self, address: SubstateId, lock_flag: LockFlag) -> Result<LockedSubstate, RuntimeError> {
        self.write_with(|state| match lock_flag {
            LockFlag::Read => state.read_lock_substate(address),
            LockFlag::Write => state.write_lock_substate(address),
        })
    }

    pub fn unlock_substate(&mut self, locked: LockedSubstate) -> Result<(), RuntimeError> {
        self.write_with(|state| state.unlock_substate(locked))
    }

    pub fn push_call_frame(&mut self, push_frame: PushCallFrame) -> Result<(), RuntimeError> {
        self.write_with(|state| {
            // If substates used in args are in scope for the current frame, we can bring then into scope for the new
            // frame
            trace!(
                 target: LOG_TARGET,
                "CALL FRAME before:\n{}",
                state.current_call_scope()?,
            );
            state.check_all_substates_in_scope(push_frame.arg_scope())?;

            let new_frame = push_frame.into_new_call_frame();
            trace!(target: LOG_TARGET, "NEW CALL FRAME:\n{}", new_frame.scope());

            state.push_frame(new_frame, limits::ENGINE_LIMITS.max_call_depth)
        })
    }

    pub fn pop_call_frame(&mut self, returned: &IndexedWellKnownTypes) -> Result<(), RuntimeError> {
        self.write_with(|state| state.pop_frame(returned))
    }

    pub fn take_last_instruction_output(&mut self) -> Option<IndexedValue> {
        self.write_with(|state| state.take_last_instruction_output())
    }

    pub fn with_workspace<F: FnOnce(&Workspace) -> R, R>(&self, f: F) -> R {
        self.read_with(|state| f(state.workspace()))
    }

    pub fn with_workspace_mut<F: FnOnce(&mut Workspace) -> R, R>(&mut self, f: F) -> R {
        self.write_with(|state| f(state.workspace_mut()))
    }

    pub fn add_fee_charge(&mut self, source: FeeSource, amount: u64) {
        if amount == 0 {
            debug!(target: LOG_TARGET, "Add fee: source: {:?}, amount: {}", source, amount);
            return;
        }

        self.write_with(|state| {
            debug!(target: LOG_TARGET, "Add fee: source: {:?}, amount: {}", source, amount);
            state.fee_state_mut().add_charge(source, amount);
        })
    }

    pub fn accumulate_wasm_points(&mut self, points: u64) {
        self.write_with(|state| state.fee_state_mut().accumulate_wasm_points(points))
    }

    pub fn accumulated_wasm_points(&self) -> u64 {
        self.read_with(|state| state.fee_state().accumulated_wasm_points())
    }

    pub fn accumulated_native_points(&self) -> u64 {
        self.read_with(|state| state.fee_state().accumulated_native_points())
    }

    /// Charges native verification work (priced in WASM-point equivalents) against the compute
    /// allowance, *before* the work is performed. Errors when the charge would exceed the
    /// allowance, so a transaction that cannot cover it traps here — having done none of the priced
    /// crypto — rather than extracting it for free. Where no allowance applies — unpriced WASM
    /// execution, or a dry run past the fee checkpoint — the charge only accumulates, so those
    /// estimates stay accurate. A dry run's fee intent is bound by the credit as a real one is,
    /// since the credit is the same figure either way.
    pub fn charge_native_execution(&mut self, points: u64) -> Result<(), RuntimeError> {
        // Hard per-transaction ceiling, independent of what the transaction pays: it bounds how far a block may
        // overshoot the propose-time execution budget, which the validation budget has to leave room for.
        let native_total = self.accumulated_native_points().saturating_add(points);
        if native_total > limits::MAX_NATIVE_POINTS_PER_TRANSACTION {
            return Err(RuntimeError::MaxNativeExecutionPointsExceeded {
                consumed_points: native_total,
                max_points: limits::MAX_NATIVE_POINTS_PER_TRANSACTION,
            });
        }
        if let Some(allowance) = self.compute_allowance() {
            let consumed = self
                .accumulated_wasm_points()
                .saturating_add(self.accumulated_native_points());
            if consumed.saturating_add(points) > allowance.points {
                if allowance.funding == ComputeFunding::FeeIntentCredit {
                    return Err(RuntimeError::FeeIntentComputeExceeded {
                        required_points: points,
                        consumed_points: consumed,
                        credit_points: allowance.points,
                    });
                }
                return Err(RuntimeError::InsufficientFeesForNativeExecution {
                    required_points: points,
                    consumed_points: consumed,
                    allowance: allowance.points,
                });
            }
        }
        self.write_with(|state| state.fee_state_mut().accumulate_native_points(points));
        Ok(())
    }

    /// Chooses the state that finalization will persist.
    ///
    /// A transaction that failed — either explicitly, or by not covering the fees charged against
    /// the working state — commits only its fee intent, so the state to persist is the fee
    /// checkpoint rather than the state execution ended on. The fee state carries over either way,
    /// so the work done before the failure is still charged for.
    ///
    /// Split out from [`Self::finalize`] so the runtime can run its modules against the chosen state
    /// before its fees are settled.
    pub fn select_finalized_state(
        &mut self,
        failure: Option<RejectReason>,
    ) -> Result<FinalizedState<TStore>, RuntimeError> {
        let total_fees_required = self.read_with(|state| state.fee_state().total_charges());
        let failure = failure.or_else(|| {
            self.read_with(|state| {
                let fee_state = state.fee_state();
                if fee_state.is_dry_run() || fee_state.is_paid_in_full() {
                    None
                } else {
                    Some(RejectReason::InsufficientFeesPaid(format!(
                        "Required fees {} but {} paid",
                        fee_state.total_charges(),
                        fee_state.total_payments()
                    )))
                }
            })
        });

        let Some(reason) = failure else {
            // Finalise will always reset the state
            return Ok(FinalizedState {
                state: self.take_working_state()?,
                outcome: FinalizeOutcome::Commit,
                reason: None,
                total_fees_required,
            });
        };

        let mut checkpoint_state = self.take_fee_checkpoint().ok_or(RuntimeError::NoFeeCheckpoint)?;
        // Preserve fee state across resets so that we can charge for fees incurred during execution before the
        // failure
        self.read_with(|state| {
            // Fee state in `state` includes the payments and charges from the fee transaction
            *checkpoint_state.fee_state_mut() = state.fee_state().clone();
        });

        Ok(FinalizedState {
            state: checkpoint_state,
            outcome: FinalizeOutcome::FeeIntentCommit,
            reason: Some(reason),
            total_fees_required,
        })
    }

    pub fn finalize(&mut self, finalized: FinalizedState<TStore>) -> Result<FinalizeResult, RuntimeError> {
        let FinalizedState {
            mut state,
            outcome,
            reason,
            total_fees_required,
        } = finalized;

        // Committing costs whatever the charges now say, and they have just been recomputed over
        // this exact state. A payment that cannot cover that commits nothing: a fee-intent commit
        // persists real state — substates, and a receipt carrying every event the intent emitted —
        // and there is no shallower checkpoint left to fall back to. Letting it through is what
        // would make an underfunded fee intent a way to write state for free.
        //
        // The commit path cannot fail here: its charges were tested against payments to reach it,
        // and recomputing them over the same state does not change them.
        let fee_state = state.fee_state();
        if !fee_state.is_dry_run() && !fee_state.is_paid_in_full() {
            // Whatever rejected the main intent is why the transaction failed and stays the reason
            // reported; the shortfall below only decides that not even the fee intent survives it.
            let reason = reason.unwrap_or_else(|| {
                RejectReason::InsufficientFeesPaid(format!(
                    "Committing requires {} but {} paid",
                    fee_state.total_charges(),
                    fee_state.total_payments()
                ))
            });

            // Nothing is persisted and nothing is taken, but what committing *would* have cost is
            // the number the payer needs to retry with — and when the main intent failed for its own
            // reason, the breakdown is the only signal that fees were the second problem. Report the
            // charges as metered, against no payment.
            let fee_receipt = FeeReceipt::builder()
                .with_cost_breakdown(state.fee_state_mut().take_fee_charges())
                .build();

            return Ok(FinalizeResult::new(
                state.transaction_hash(),
                state.take_logs(),
                // Events describe state changes, and none of them happened.
                Vec::new(),
                TransactionResult::Reject(reason),
                fee_receipt,
            )
            .with_total_fees_required(total_fees_required));
        }

        // Resolve the transfers to the fee pool resource and vault refunds
        let mut substates_to_persist = state.take_mutated_substates();
        let fee_receipt = state.finalize_fees_and_refunds(&mut substates_to_persist)?;

        let downed_utxos = state.take_downed_utxos();
        let downed_confidential_outputs = state.take_downed_confidential_outputs();
        let fee_withdrawals = state.take_validator_fee_withdrawals();

        let mut diff = state.generate_substate_diff(
            substates_to_persist,
            downed_utxos,
            downed_confidential_outputs,
            fee_withdrawals,
        )?;
        let transaction_receipt = state.finalize_transaction_receipt(outcome, &diff, fee_receipt.clone())?;
        diff.up(
            SubstateId::TransactionReceipt(state.transaction_hash().into()),
            Substate::new(0, transaction_receipt),
        );

        let result = match reason {
            Some(reason) => TransactionResult::AcceptFeeRejectRest(diff, reason),
            None => TransactionResult::Accept(diff),
        };

        Ok(FinalizeResult::new(
            state.transaction_hash(),
            state.take_logs(),
            state.take_events(),
            result,
            fee_receipt,
        )
        .with_total_fees_required(total_fees_required))
    }

    fn take_fee_checkpoint(&mut self) -> Option<WorkingState<TStore>> {
        self.fee_checkpoint.take()
    }

    fn take_working_state(&mut self) -> Result<WorkingState<TStore>, RuntimeError> {
        self.working_state.take().ok_or_else(|| RuntimeError::InvariantError {
            function: "StateTracker::take_working_state",
            details: "Working state has already been taken (double finalize?)".to_string(),
        })
    }

    /// The working state, as a module may charge against it.
    ///
    /// The fee module uses this to compute its finalization charges before the state to persist has
    /// been chosen; once it has been, the same computation runs against that state directly.
    pub fn chargeable_state(&mut self) -> ChargeableState<'_, TStore> {
        ChargeableState::new(
            self.working_state
                .as_mut()
                .expect("BUG: chargeable_state called after finalize consumed working state"),
        )
    }

    pub fn are_fees_paid_in_full(&self) -> bool {
        self.read_with(|state| {
            let total_payments = state.fee_state().total_payments();
            let total_charges = state.fee_state().total_charges();
            total_payments >= total_charges
        })
    }

    pub fn total_fee_payments(&self) -> u64 {
        self.read_with(|state| state.fee_state().total_payments())
    }

    /// The payment the charges metered so far require. This is the figure a rejected payer has to
    /// raise their fee to.
    pub fn required_fee_payment(&self) -> u64 {
        self.read_with(|state| state.fee_state().total_charges())
    }

    /// A copy of the charges metered so far.
    pub fn fee_charges(&self) -> FeeBreakdown {
        self.read_with(|state| state.fee_state().fee_charges().clone())
    }

    pub fn total_fee_charges(&self) -> u64 {
        self.read_with(|state| state.fee_state().total_charges())
    }

    pub fn is_fee_state_dry_run(&self) -> bool {
        self.read_with(|state| state.fee_state().is_dry_run())
    }

    /// The write mode of the current call frame. Used by the engine's core sandbox enforcement to deny the few
    /// effectful host ops that bypass the lock layer.
    pub fn current_frame_write_mode(&self) -> FrameWriteMode {
        self.working_state
            .as_ref()
            .map(|state| state.current_frame_write_mode())
            .unwrap_or(FrameWriteMode::Full)
    }

    pub(super) fn read_with<R, F: FnOnce(&WorkingState<TStore>) -> R>(&self, f: F) -> R {
        f(self
            .working_state
            .as_ref()
            .expect("BUG: read_with called after finalize consumed working state"))
    }

    pub(super) fn write_with<R, F: FnOnce(&mut WorkingState<TStore>) -> R>(&mut self, f: F) -> R {
        f(self
            .working_state
            .as_mut()
            .expect("BUG: write_with called after finalize consumed working state"))
    }

    pub(super) fn is_fee_intent_checkpointed(&self) -> bool {
        self.fee_checkpoint.is_some()
    }

    /// Accounts one more stealth transfer against [`limits::StealthLimits::max_fee_intent_transfers`]; a no-op once
    /// the fee intent is over. Called before the transfer's native cost is charged and before any of its crypto runs,
    /// so an over-cap fee intent is rejected without the work being performed.
    ///
    /// Counts every transfer the fee intent performs, whether from a `StealthTransfer` instruction or from a template
    /// calling `ResourceManager::stealth_transfer` — both reach this through `RuntimeInterfaceImpl::stealth_transfer`.
    /// Counting instructions alone would leave the WASM route uncapped, and so make the more expensive route the way
    /// to exceed the limit.
    pub(super) fn account_fee_intent_stealth_transfer(&mut self) -> Result<(), RuntimeError> {
        if self.is_fee_intent_checkpointed() {
            return Ok(());
        }
        let max_transfers = limits::STEALTH_LIMITS.max_fee_intent_transfers;
        if self.fee_intent_stealth_transfers + 1 > max_transfers {
            return Err(ArgumentValidationError::MaxFeeIntentStealthTransfersExceeded { max_transfers }.into());
        }
        self.fee_intent_stealth_transfers += 1;
        Ok(())
    }
}

impl<TStore: StateReader + Clone> StateTracker<TStore> {
    pub fn fee_checkpoint(&mut self) -> Result<(), RuntimeError> {
        let state = self.write_with(|state| {
            // Check that the checkpoint is in a valid state
            state.validate_finalized()?;
            let checkpoint_state = state.clone();

            // After checkpointing, the main intent has a cleared workspace
            state.workspace_mut().clear_items();
            let proofs = state.workspace_mut().drain_all_proofs();
            for proof_id in proofs {
                state.drop_proof(proof_id)?;
            }
            Ok::<_, RuntimeError>(checkpoint_state)
        })?;

        self.fee_checkpoint = Some(state);
        Ok(())
    }
}
