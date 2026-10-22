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

use std::{
    cell::{Cell, RefCell},
    collections::HashSet,
};

use log::*;
use ootle_network::Network;
use tari_engine_types::{
    SubstateVersion,
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
        fee_state::FeeState,
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
    /// On a fee-intent commit, the fee state as the fee checkpoint left it: what the fee intent
    /// alone was charged and paid. `state` carries the whole transaction's fee state, main intent
    /// included.
    fee_intent_fees: Option<FeeState>,
}

impl<TStore> FinalizedState<TStore> {
    pub fn chargeable_state(&mut self) -> ChargeableState<'_, TStore> {
        ChargeableState::new(&mut self.state)
    }

    /// The persisted state as the fee intent alone is charged for it. `None` unless this is a
    /// fee-intent commit.
    pub fn fee_intent_chargeable_state(&mut self) -> Option<ChargeableState<'_, TStore>> {
        let fee_state = self.fee_intent_fees.as_mut()?;
        Some(ChargeableState::with_fee_state(&mut self.state, fee_state))
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

/// What a transaction's declared inputs weigh as every executing validator loads them and pledges
/// them across shard groups. Measured before execution starts, from the declared inputs alone, so it
/// is charged whatever the transaction goes on to do with them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeclaredInputLoad {
    /// Encoded bytes of the values of the declared inputs, global ones excepted.
    pub bytes: u64,
    /// The other shard groups the declared inputs are pledged to.
    pub fanout: u32,
}

/// The transaction's execution state, shared by every frame that runs under it.
///
/// A nested call re-enters the engine against the same tracker the outer frame is running on, so
/// mutation goes through `&self` and an internal borrow. Every such borrow closes before the
/// statement that took it ends, which is what keeps a nested call from finding one open.
#[derive(Debug)]
pub struct StateTracker<TStore> {
    working_state: RefCell<Option<WorkingState<TStore>>>,
    fee_checkpoint: RefCell<Option<WorkingState<TStore>>>,
    transaction_weight: TransactionWeight,
    declared_input_load: DeclaredInputLoad,
    wasm_metering_rate: WasmMeteringRate,
    /// Stealth transfers performed so far in the fee intent. Lives here rather than in `WorkingState` because the fee
    /// intent is delimited by `fee_checkpoint`, and because the checkpoint clones the working state — a count held
    /// there would be duplicated into the clone.
    fee_intent_stealth_transfers: Cell<usize>,
}

impl<TStore: StateReader> StateTracker<TStore> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        state_store: TStore,
        virtual_substates: VirtualSubstates,
        initial_call_scope: CallScope,
        read_declared_inputs: HashSet<SubstateId>,
        transaction_hash: Hash32,
        intent_commitment: Hash32,
        transaction_weight: TransactionWeight,
        declared_input_load: DeclaredInputLoad,
        wasm_metering_rate: WasmMeteringRate,
        burn_rate: ExhaustBurnRate,
        network: Network,
        dry_run: bool,
    ) -> Self {
        Self {
            working_state: RefCell::new(Some(WorkingState::new(
                state_store,
                virtual_substates,
                initial_call_scope,
                read_declared_inputs,
                transaction_hash,
                intent_commitment,
                burn_rate,
                network,
                dry_run,
            ))),
            fee_checkpoint: RefCell::new(None),
            transaction_weight,
            declared_input_load,
            wasm_metering_rate,
            fee_intent_stealth_transfers: Cell::new(0),
        }
    }

    pub fn get_transaction_weight(&self) -> TransactionWeight {
        self.transaction_weight
    }

    pub fn declared_input_load(&self) -> DeclaredInputLoad {
        self.declared_input_load
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
    /// costs, and that share of the payment is already spoken for; funding compute out of the full
    /// payment would run work nothing is left to pay for.
    ///
    /// That payment-funded figure is measured against the charges *standing when it is asked*.
    /// Anything charged after the last call — instructions that follow the final invocation — is
    /// outside it, so the total can end a little past the payment. The fallback commit is decided
    /// on the fee intent's own charges, so it still settles and collects the whole payment.
    pub fn compute_allowance(&self) -> Option<ComputeAllowance> {
        let rate = self.wasm_metering_rate;
        let is_fee_intent = self.fee_checkpoint.borrow().is_none();
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
        self.write_with(|state| {
            // TODO: epoch_hash is a bad source of entropy. Agreeing on randomness at a consensus level is challenging
            // in multi-sharded consensus.
            let epoch_hash = state.get_current_epoch_hash()?;
            let bytes = state.id_provider()?.get_random_bytes(&epoch_hash, length)?;
            Ok(bytes)
        })
    }

    pub fn add_event(&self, event: Event) -> Result<(), RuntimeError> {
        debug!(target: LOG_TARGET, "Emit: {event}");
        self.write_with(|state| state.push_event(event))
    }

    /// Adds `log` to the transaction's logs, handing it to `on_added` only if it was accepted.
    pub fn add_log<F: FnOnce(&LogEntry)>(&self, log: LogEntry, on_added: F) -> Result<(), RuntimeError> {
        self.write_with(|state| state.push_log(log).map(on_added))
    }

    /// Returns `true` the first time a given template address is seen within this transaction's
    /// state, `false` thereafter. Callers use this to dedupe `FeeSource::TemplateLoad` charges:
    /// the validator pays the cold compile/deserialise cost at most once per template per
    /// process, so charging it on every entry over-bills the user.
    pub fn record_template_load_charge(&self, address: TemplateAddress) -> bool {
        self.write_with(|state| state.record_template_load_charge(address))
    }

    pub fn take_events(&self) -> Vec<Event> {
        self.write_with(|state| state.take_events())
    }

    pub fn get_template_address(&self) -> Result<TemplateAddress, RuntimeError> {
        self.read_with(|state| state.current_template().copied())
    }

    pub fn get_template_module_name(&self) -> Result<String, RuntimeError> {
        self.read_with(|state| state.current_template_name().map(ToOwned::to_owned))
    }

    pub fn new_component(
        &self,
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

    pub fn lock_substate(&self, address: SubstateId, lock_flag: LockFlag) -> Result<LockedSubstate, RuntimeError> {
        self.write_with(|state| match lock_flag {
            LockFlag::Read => state.read_lock_substate(address),
            LockFlag::Write => state.write_lock_substate(address),
        })
    }

    pub fn unlock_substate(&self, locked: LockedSubstate) -> Result<(), RuntimeError> {
        self.write_with(|state| state.unlock_substate(locked))
    }

    pub fn push_call_frame(&self, push_frame: PushCallFrame) -> Result<(), RuntimeError> {
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

    pub fn pop_call_frame(&self, returned: &IndexedWellKnownTypes) -> Result<(), RuntimeError> {
        self.write_with(|state| state.pop_frame(returned))
    }

    pub fn take_last_instruction_output(&self) -> Option<IndexedValue> {
        self.write_with(|state| state.take_last_instruction_output())
    }

    pub fn with_workspace<F: FnOnce(&Workspace) -> R, R>(&self, f: F) -> R {
        self.read_with(|state| f(state.workspace()))
    }

    pub fn with_workspace_mut<F: FnOnce(&mut Workspace) -> R, R>(&self, f: F) -> R {
        self.write_with(|state| f(state.workspace_mut()))
    }

    pub fn add_fee_charge(&self, source: FeeSource, amount: u64) {
        if amount == 0 {
            debug!(target: LOG_TARGET, "Add fee: source: {:?}, amount: {}", source, amount);
            return;
        }

        self.write_with(|state| {
            debug!(target: LOG_TARGET, "Add fee: source: {:?}, amount: {}", source, amount);
            state.fee_state_mut().add_charge(source, amount);
        })
    }

    pub fn accumulate_wasm_points(&self, points: u64) {
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
    pub fn charge_native_execution(&self, points: u64) -> Result<(), RuntimeError> {
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

    /// The most points [`Self::charge_native_execution`] would accept in one charge now, so that work
    /// priced as it runs can be bounded to what the transaction can pay for before it starts.
    pub fn native_execution_headroom(&self) -> u64 {
        let native = self.accumulated_native_points();
        let cap_remaining = limits::MAX_NATIVE_POINTS_PER_TRANSACTION.saturating_sub(native);
        match self.compute_allowance() {
            Some(allowance) => {
                let consumed = self.accumulated_wasm_points().saturating_add(native);
                cap_remaining.min(allowance.points.saturating_sub(consumed))
            },
            None => cap_remaining,
        }
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
        &self,
        failure: Option<RejectReason>,
    ) -> Result<FinalizedState<TStore>, RuntimeError> {
        let total_fees_required = self.read_with(|state| state.fee_state().total_charges());
        let failure = match failure {
            Some(reason) => Some(reason),
            None => self
                .write_with(|state| state.enforce_output_size_limit())
                .err()
                .map(|err| err.to_reject_reason(None)),
        };
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
                fee_intent_fees: None,
            });
        };

        let mut checkpoint_state = self.take_fee_checkpoint().ok_or(RuntimeError::NoFeeCheckpoint)?;
        // Preserve fee state across resets so that we can charge for fees incurred during execution before the
        // failure
        let fee_intent_fees = self.read_with(|state| {
            // Fee state in `state` includes the payments and charges from the fee transaction
            std::mem::replace(checkpoint_state.fee_state_mut(), state.fee_state().clone())
        });

        Ok(FinalizedState {
            state: checkpoint_state,
            outcome: FinalizeOutcome::FeeIntentCommit,
            reason: Some(reason),
            total_fees_required,
            fee_intent_fees: Some(fee_intent_fees),
        })
    }

    pub fn finalize(&self, finalized: FinalizedState<TStore>) -> Result<FinalizeResult, RuntimeError> {
        let FinalizedState {
            mut state,
            outcome,
            reason,
            total_fees_required,
            fee_intent_fees,
        } = finalized;

        // Whether this state may commit turns on what the commit itself costs. On a full commit
        // that is every charge, and the commit path cannot fail here: its charges were tested
        // against payments to reach it, and recomputing them over the same state does not change
        // them.
        //
        // On a fee-intent commit it is what the fee intent alone is charged for the state it
        // persists. A fee intent that cannot cover that commits nothing: a fee-intent commit
        // persists real state — substates, and a receipt carrying every event the intent emitted —
        // and there is no shallower checkpoint left to fall back to. Letting it through is what
        // would make an underfunded fee intent a way to write state for free.
        //
        // The compute allowance holds the main intent's charges inside the payment only up to the
        // last allowance it sized, so charges accrued past that can leave the total a little over
        // the payment. A fee intent that paid for itself commits regardless, and settlement collects
        // the whole payment against the larger total.
        let commit_fees = fee_intent_fees.as_ref().unwrap_or_else(|| state.fee_state());
        if !commit_fees.is_dry_run() && !commit_fees.is_paid_in_full() {
            let fee_state = state.fee_state();
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

        let downed = state.take_downed();
        let fee_withdrawals = state.take_validator_fee_withdrawals();

        let mut diff = state.generate_substate_diff(substates_to_persist, downed, fee_withdrawals)?;
        let transaction_receipt = state.finalize_transaction_receipt(outcome, &diff, fee_receipt.clone())?;
        diff.up(
            SubstateId::TransactionReceipt(state.transaction_hash().into()),
            Substate::new(SubstateVersion::ZERO, transaction_receipt),
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

    fn take_fee_checkpoint(&self) -> Option<WorkingState<TStore>> {
        self.fee_checkpoint.borrow_mut().take()
    }

    fn take_working_state(&self) -> Result<WorkingState<TStore>, RuntimeError> {
        self.working_state
            .borrow_mut()
            .take()
            .ok_or_else(|| RuntimeError::InvariantError {
                function: "StateTracker::take_working_state",
                details: "Working state has already been taken (double finalize?)".to_string(),
            })
    }

    /// The working state, as a module may charge against it.
    ///
    /// The fee module uses this to compute its finalization charges before the state to persist has
    /// been chosen; once it has been, the same computation runs against that state directly.
    pub fn with_chargeable_state<R, F: FnOnce(&mut ChargeableState<'_, TStore>) -> R>(&self, f: F) -> R {
        f(&mut ChargeableState::new(
            self.working_state
                .borrow_mut()
                .as_mut()
                .expect("BUG: with_chargeable_state called after finalize consumed working state"),
        ))
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
            .borrow()
            .as_ref()
            .map(|state| state.current_frame_write_mode())
            .unwrap_or(FrameWriteMode::Full)
    }

    pub(super) fn read_with<R, F: FnOnce(&WorkingState<TStore>) -> R>(&self, f: F) -> R {
        f(self
            .working_state
            .borrow()
            .as_ref()
            .expect("BUG: read_with called after finalize consumed working state"))
    }

    pub(super) fn write_with<R, F: FnOnce(&mut WorkingState<TStore>) -> R>(&self, f: F) -> R {
        f(self
            .working_state
            .borrow_mut()
            .as_mut()
            .expect("BUG: write_with called after finalize consumed working state"))
    }

    pub(super) fn is_fee_intent_checkpointed(&self) -> bool {
        self.fee_checkpoint.borrow().is_some()
    }

    /// Accounts one more stealth transfer against [`limits::StealthLimits::max_fee_intent_transfers`]; a no-op once
    /// the fee intent is over. Called before the transfer's native cost is charged and before any of its crypto runs,
    /// so an over-cap fee intent is rejected without the work being performed.
    ///
    /// Counts every transfer the fee intent performs, whether from a `StealthTransfer` instruction or from a template
    /// calling `ResourceManager::stealth_transfer` — both reach this through `RuntimeInterfaceImpl::stealth_transfer`.
    /// Counting instructions alone would leave the WASM route uncapped, and so make the more expensive route the way
    /// to exceed the limit.
    pub(super) fn account_fee_intent_stealth_transfer(&self) -> Result<(), RuntimeError> {
        if self.is_fee_intent_checkpointed() {
            return Ok(());
        }
        let max_transfers = limits::STEALTH_LIMITS.max_fee_intent_transfers;
        if self.fee_intent_stealth_transfers.get() + 1 > max_transfers {
            return Err(ArgumentValidationError::MaxFeeIntentStealthTransfersExceeded { max_transfers }.into());
        }
        self.fee_intent_stealth_transfers
            .set(self.fee_intent_stealth_transfers.get() + 1);
        Ok(())
    }
}

impl<TStore: StateReader + Clone> StateTracker<TStore> {
    pub fn fee_checkpoint(&self) -> Result<(), RuntimeError> {
        let state = self.write_with(|state| {
            // Check that the checkpoint is in a valid state. It is what commits if the main intent
            // fails, so it is held to every limit a full commit is.
            state.validate_finalized()?;
            state.enforce_output_size_limit()?;
            let checkpoint_state = state.clone();

            // After checkpointing, the main intent has a cleared workspace
            state.workspace_mut().clear_items();
            let proofs = state.workspace_mut().take_all_proofs();
            for proof_id in proofs {
                state.drop_proof(proof_id)?;
            }
            Ok::<_, RuntimeError>(checkpoint_state)
        })?;

        *self.fee_checkpoint.borrow_mut() = Some(state);
        Ok(())
    }
}
