//    Copyright 2023 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::sync::Arc;

use ootle_network::Network;
use tari_engine::{
    executables::Executable,
    fees::{FeeModule, FeeTable, WasmMeteringRate},
    runtime::{AuthParams, RuntimeModule},
    state_store::{StateReader, StateStoreError},
    template::LoadedTemplate,
    traits::ClaimProofVerifier,
    transaction::{ModulesCollection, TransactionError, TransactionProcessor},
};
use tari_engine_types::{
    commit_result::ExecuteResult,
    fees::{ExhaustBurnRate, FeeScale, FeeScaleSchedule},
    substate::Substate,
    virtual_substate::VirtualSubstates,
};
use tari_ootle_common_types::{
    Epoch,
    SubstateLockType,
    SubstateRequirement,
    VersionedSubstateId,
    services::template_provider::TemplateProvider,
};
use tari_ootle_storage::consensus_models::VersionedSubstateIdLockIntent;
use tari_ootle_transaction::Transaction;
use tari_template_lib::types::NonFungibleAddress;

const _LOG_TARGET: &str = "tari::ootle::transaction_executor";

pub trait TransactionExecutor<TStore> {
    type Error: std::error::Error + Send + Sync + 'static;

    /// Executes `transaction`, resolving every template it calls from `template_provider`.
    ///
    /// `input_fanout` is [`Transaction::input_fanout`] for the shard layout the transaction executes
    /// under.
    fn execute<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
        &self,
        transaction: &Transaction,
        template_provider: TTemplateProvider,
        state_store: TStore,
        virtual_substates: VirtualSubstates,
        burn_rate: ExhaustBurnRate,
        input_fanout: u32,
    ) -> Result<ExecutionOutput, Self::Error>;
}

#[derive(Debug, Clone)]
pub struct ExecutionOutput {
    pub result: ExecuteResult,
}

impl ExecutionOutput {
    pub fn resolve_input_locks<'a, I: IntoIterator<Item = (&'a SubstateRequirement, &'a Substate)>>(
        &self,
        inputs: I,
    ) -> Vec<VersionedSubstateIdLockIntent> {
        if let Some(diff) = self.result.finalize.any_accept() {
            inputs
                .into_iter()
                .map(|(substate_req, substate)| {
                    let requested_specific_version = substate_req.version().is_some();
                    let id = substate_req.substate_id();
                    // A fee pool a transaction withdraws from is written by consensus rather than through the
                    // diff, so the withdrawal is what marks it written.
                    let is_withdrawn_from = id
                        .as_validator_fee_pool_address()
                        .is_some_and(|address| diff.validator_fee_withdrawals().iter().any(|w| w.address == address));
                    let lock_flag = if is_withdrawn_from || diff.down_iter().any(|(down_id, _)| down_id == id) {
                        // Update all inputs that were DOWNed to be write locked
                        SubstateLockType::Write
                    } else {
                        // Any input not downed, gets a read lock
                        SubstateLockType::Read
                    };
                    VersionedSubstateIdLockIntent::new(
                        VersionedSubstateId::new(substate_req.substate_id().clone(), substate.version()),
                        lock_flag,
                        requested_specific_version,
                    )
                    .with_pledged_value(substate.substate_value())
                })
                .collect()
        } else {
            // TODO: we might want to have a SubstateLockFlag::None for rejected transactions so that we still know the
            // shards involved but do not lock them. We dont actually lock anything for rejected transactions anyway.
            inputs
                .into_iter()
                .map(|(substate_req, substate)| {
                    VersionedSubstateIdLockIntent::new(
                        VersionedSubstateId::new(substate_req.substate_id().clone(), substate.version()),
                        SubstateLockType::Read,
                        true,
                    )
                })
                .collect()
        }
    }
}

#[derive(Clone)]
pub struct TariTransactionProcessor<TStore> {
    network: Network,
    modules: ModulesCollection<TStore>,
    dry_run: bool,
    claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
    wasm_metering_rate: WasmMeteringRate,
}

impl<TStore: StateReader + 'static> TariTransactionProcessor<TStore> {
    pub fn new(
        network: Network,
        fee_table: FeeTable,
        dry_run: bool,
        claim_burn_proof_verifier: Arc<dyn ClaimProofVerifier + Send + Sync + 'static>,
    ) -> Self {
        let wasm_metering_rate = WasmMeteringRate::from_fee_table(&fee_table);
        // The fee module is stateless with respect to the burn rate and dry-run flag: both are seeded onto the fee
        // state per-execution, so the module list is built once and shared.
        let modules = vec![Box::new(FeeModule::new(0, fee_table)) as Box<dyn RuntimeModule<TStore>>];
        Self {
            network,
            modules: Arc::from(modules),
            dry_run,
            claim_burn_proof_verifier,
            wasm_metering_rate,
        }
    }
}

impl<TStore: StateReader + Clone + 'static> TransactionExecutor<TStore> for TariTransactionProcessor<TStore> {
    type Error = TransactionProcessorError;

    fn execute<TTemplateProvider: TemplateProvider<Template = LoadedTemplate>>(
        &self,
        transaction: &Transaction,
        template_provider: TTemplateProvider,
        state_store: TStore,
        virtual_substates: VirtualSubstates,
        burn_rate: ExhaustBurnRate,
        input_fanout: u32,
    ) -> Result<ExecutionOutput, Self::Error> {
        // Include signature public key badges for all transaction signers in the initial auth scope
        // NOTE: we assume all signatures have already been validated.
        let initial_ownership_proofs = transaction
            .signers_iter()
            .map(|pk| NonFungibleAddress::from_public_key(*pk))
            .collect();
        let auth_params = AuthParams {
            initial_ownership_proofs,
        };
        // A transaction executed with no current epoch cannot finalize, since its receipt names the
        // epoch, so the scale it would otherwise be charged at is never billed.
        let fee_scale = virtual_substates
            .current_epoch()
            .map_or(FeeScale::ONE, |epoch| FeeScaleSchedule::at(self.network, Epoch(epoch)));

        let processor = TransactionProcessor::new(
            Arc::new(template_provider),
            state_store,
            auth_params,
            virtual_substates,
            self.modules.clone(),
            self.claim_burn_proof_verifier.clone(),
            self.wasm_metering_rate,
            burn_rate,
            input_fanout,
            fee_scale,
            self.network,
            self.dry_run,
        );
        let result = processor.execute(transaction.clone())?;

        Ok(ExecutionOutput { result })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TransactionProcessorError {
    #[error(transparent)]
    TransactionError(#[from] TransactionError),
    #[error(transparent)]
    StateStoreError(#[from] StateStoreError),
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tari_engine_types::{
        ValidatorFeePool,
        ValidatorFeeWithdrawal,
        commit_result::{ExecuteResult, FinalizeResult, TransactionResult},
        fees::FeeReceipt,
        substate::{SubstateDiff, SubstateId},
    };
    use tari_ootle_common_types::{LockIntent, SubstateVersion};
    use tari_template_lib::types::{Hash32, ValidatorFeePoolAddress};

    use super::*;

    fn output(diff: SubstateDiff) -> ExecutionOutput {
        ExecutionOutput {
            result: ExecuteResult {
                finalize: FinalizeResult::new(
                    Hash32::default(),
                    vec![],
                    vec![],
                    TransactionResult::Accept(diff),
                    FeeReceipt::default(),
                ),
                execution_time: Duration::default(),
                execute_epoch: None,
                wasm_execution_points: 0,
                native_execution_points: 0,
            },
        }
    }

    #[test]
    fn a_withdrawn_fee_pool_is_write_locked() {
        let withdrawn = ValidatorFeePoolAddress::from_array([1; 32]);
        let untouched = ValidatorFeePoolAddress::from_array([2; 32]);
        let mut diff = SubstateDiff::new();
        diff.set_once_fee_withdrawals(vec![ValidatorFeeWithdrawal {
            address: withdrawn,
            amount: 10,
        }]);

        let pool = Substate::new(SubstateVersion::new(3), ValidatorFeePool::new(Default::default(), 100));
        let withdrawn_req = SubstateRequirement::unversioned(SubstateId::ValidatorFeePool(withdrawn));
        let untouched_req = SubstateRequirement::unversioned(SubstateId::ValidatorFeePool(untouched));

        let locks = output(diff).resolve_input_locks([(&withdrawn_req, &pool), (&untouched_req, &pool)]);

        assert_eq!(locks[0].lock_type(), SubstateLockType::Write);
        assert_eq!(locks[1].lock_type(), SubstateLockType::Read);
    }
}
