//  Copyright 2023. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{collections::HashMap, sync::Arc};

use log::info;
use tari_comms::types::CommsPublicKey;
use tari_dan_app_utilities::{
    payload_processor::TariDanPayloadProcessor,
    template_manager::implementation::TemplateManager,
};
use tari_dan_common_types::{optional::IsNotFoundError, Epoch, ObjectPledge, PayloadId, ShardId, SubstateState};
use tari_dan_core::services::PayloadProcessor;
use tari_dan_engine::{fees::FeeTable, runtime::ConsensusContext};
use tari_dan_storage::models::{Payload, TariDanPayload};
use tari_engine_types::commit_result::ExecuteResult;
use tari_epoch_manager::{base_layer::EpochManagerError, EpochManager};
use tari_indexer_lib::{substate_scanner::SubstateScanner, transaction_autofiller::TransactionAutofiller};
use tari_transaction::Transaction;
use tari_validator_node_rpc::client::{ValidatorNodeClientFactory, ValidatorNodeRpcClient};
use tokio::task;

use crate::dry_run::error::DryRunTransactionProcessorError;

const LOG_TARGET: &str = "tari::indexer::dry_run_transaction_processor";

pub struct DryRunTransactionProcessor<TEpochManager, TClientFactory> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    transaction_autofiller: TransactionAutofiller<TEpochManager, TClientFactory>,
    template_manager: TemplateManager,
}

impl<TEpochManager, TClientFactory> DryRunTransactionProcessor<TEpochManager, TClientFactory>
where
    TEpochManager: EpochManager<CommsPublicKey, Error = EpochManagerError>,
    TClientFactory: ValidatorNodeClientFactory<Addr = CommsPublicKey>,
    <TClientFactory::Client as ValidatorNodeRpcClient>::Error: IsNotFoundError,
{
    pub fn new(
        epoch_manager: TEpochManager,
        client_provider: TClientFactory,
        substate_scanner: Arc<SubstateScanner<TEpochManager, TClientFactory>>,
        template_manager: TemplateManager,
    ) -> Self {
        let transaction_autofiller = TransactionAutofiller::new(substate_scanner);

        Self {
            epoch_manager,
            client_provider,
            transaction_autofiller,
            template_manager,
        }
    }

    pub async fn process_transaction(
        &self,
        transaction: &Transaction,
    ) -> Result<ExecuteResult, DryRunTransactionProcessorError> {
        info!(target: LOG_TARGET, "process_transaction: {}", transaction.hash());

        // automatically scan the inputs and add all related involved objects
        // note that this operation does not alter the transaction hash
        let transaction = self.transaction_autofiller.autofill_transaction(transaction).await?;

        let payload = TariDanPayload::new(transaction.clone());
        let epoch = self.epoch_manager.current_epoch().await?;
        let pledges = self.get_shard_pledges(&payload, &epoch).await?;
        let payload_processor = self.build_payload_processor(&transaction);

        // execute the payload in the WASM engine and return the result
        let consensus_context = Self::get_consensus_context(&epoch).await?;
        let result = task::block_in_place(|| payload_processor.process_payload(payload, pledges, consensus_context))?;

        Ok(result)
    }

    fn build_payload_processor(&self, transaction: &Transaction) -> TariDanPayloadProcessor<TemplateManager> {
        // simulate fees if the transaction requires it
        let fee_table = if Self::transaction_includes_fees(transaction) {
            // TODO: should match the VN fee table, should the fee table values be a consensus constant?
            FeeTable::new(1, 1)
        } else {
            FeeTable::zero_rated()
        };

        TariDanPayloadProcessor::new(self.template_manager.clone(), fee_table)
    }

    fn transaction_includes_fees(transaction: &Transaction) -> bool {
        !transaction.fee_instructions().is_empty()
    }

    async fn get_shard_pledges(
        &self,
        payload: &TariDanPayload,
        epoch: &Epoch,
    ) -> Result<HashMap<ShardId, ObjectPledge>, DryRunTransactionProcessorError> {
        let mut shard_pledges = HashMap::new();

        // TODO: spawn a tokio task per pledge for better performance?
        for shard_id in payload.involved_shards() {
            let pledge = self.get_shard_pledge(shard_id, payload.to_id(), *epoch).await?;
            shard_pledges.insert(shard_id, pledge);
        }

        Ok(shard_pledges)
    }

    pub async fn get_shard_pledge(
        &self,
        shard_id: ShardId,
        payload_id: PayloadId,
        epoch: Epoch,
    ) -> Result<ObjectPledge, DryRunTransactionProcessorError> {
        let committee = self.epoch_manager.get_committee(epoch, shard_id).await?;

        for vn_public_key in committee.members {
            // build a client with the VN
            let mut sync_vn_client = self.client_provider.create_client(&vn_public_key);

            match sync_vn_client.get_shard_pledge(&shard_id).await {
                Ok(pledge) => {
                    return Ok(pledge);
                },
                Err(e) => {
                    info!(target: LOG_TARGET, "Unable to get pledge from peer: {} ", e.to_string());
                    // we do not stop when an individual VN does not respond correctly, we try all Vns
                    continue;
                },
            };
        }

        // The shard does not exist on any VN, so we pledge it to be created in this payload
        Ok(ObjectPledge {
            shard_id,
            pledged_to_payload: payload_id,
            current_state: SubstateState::DoesNotExist,
        })
    }

    async fn get_consensus_context(epoch: &Epoch) -> Result<ConsensusContext, DryRunTransactionProcessorError> {
        let current_epoch = epoch.as_u64();
        let consensus_context = ConsensusContext { current_epoch };
        Ok(consensus_context)
    }
}
