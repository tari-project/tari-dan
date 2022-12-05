use std::collections::HashMap;

use tari_dan_common_types::{ObjectPledge, ShardId};
use tari_dan_core::{
    models::{Payload, TariDanPayload},
    services::{epoch_manager::EpochManager, PayloadProcessor},
    storage::shard_store::{ShardStore, ShardStoreTransaction},
};
use tari_dan_engine::transaction::Transaction;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStore;
use tari_engine_types::commit_result::FinalizeResult;

use crate::{
    p2p::services::{epoch_manager::handle::EpochManagerHandle, template_manager::TemplateManager},
    payload_processor::TariDanPayloadProcessor,
};

#[derive(Clone)]
pub struct DryRunTransactionProcessor {
    /// The epoch manager
    epoch_manager: EpochManagerHandle,
    /// The payload processor. This determines whether a payload proposal results in an accepted or rejected vote.
    payload_processor: TariDanPayloadProcessor<TemplateManager>,
    /// Store used to persist consensus state.
    shard_store: SqliteShardStore,
}

impl DryRunTransactionProcessor {
    pub fn new(
        epoch_manager: EpochManagerHandle,
        payload_processor: TariDanPayloadProcessor<TemplateManager>,
        shard_store: SqliteShardStore,
    ) -> Self {
        Self {
            epoch_manager,
            payload_processor,
            shard_store,
        }
    }

    // TODO: create a error enum
    pub async fn process_transaction(&self, transaction: Transaction) -> Result<FinalizeResult, String> {
        // get the list of involved shards for the transaction
        let payload = TariDanPayload::new(transaction.clone());
        let involved_shards = payload.involved_shards();

        // get the local shard state
        let _epoch = self.epoch_manager.current_epoch().await.map_err(|e| e.to_string());

        // get the pledges for all local shards
        let shard_pledges = self.get_local_pledges(involved_shards).await?;

        // TODO: get non local shard pledges

        let result = self
            .payload_processor
            .process_payload(payload, shard_pledges)
            .map_err(|e| e.to_string())?;
        Ok(result)
    }

    async fn get_local_pledges(
        &self,
        involved_shards: Vec<ShardId>,
    ) -> Result<HashMap<ShardId, Option<ObjectPledge>>, String> {
        dbg!(&involved_shards);
        let tx = self.shard_store.create_tx().unwrap();
        let inventory = tx.get_state_inventory().unwrap();
        dbg!(&inventory);

        let local_shard_ids: Vec<ShardId> = involved_shards.into_iter().filter(|s| inventory.contains(s)).collect();
        dbg!(&local_shard_ids);
        let mut local_pledges = HashMap::new();
        for shard_id in local_shard_ids {
            // TODO: create a DB method to get the substates of a list of shards
            let substate_data = tx
                .get_substate_states(shard_id, shard_id, &[])
                .unwrap()
                .first()
                .unwrap()
                .clone();
            dbg!(&substate_data);
            let local_pledge = ObjectPledge {
                shard_id,
                current_state: substate_data.substate().clone(),
                pledged_to_payload: substate_data.payload_id(),
                pledged_until: substate_data.height(),
            };
            local_pledges.insert(shard_id, Some(local_pledge));
        }
        dbg!(&local_pledges);
        Ok(local_pledges)
    }
}

// let mut tx = self.shard_store.create_tx()?;
// let high_qc = tx.get_high_qc_for(shard).optional()?.unwrap_or_else(|| {
// TODO: sign genesis
// QuorumCertificate::genesis(epoch)
// });
//
// let committee = self.epoch_manager.get_committee(epoch, shard).await?;
// let leader = self.leader_strategy.get_leader(&committee, payload_id, shard, 0);
//
//
// GET THE LOCAL SHARD STATE
// fn pledge_object(
// &mut self,
// shard: ShardId,
// payload: PayloadId,
// change: SubstateChange,
// current_height: NodeHeight,
// ) -> Result<ObjectPledge, StorageError>;
// let local_pledge = tx.pledge_object(shard, payload_id, change, parent_leaf_node.height())?;
//
//
// GET EXTERNAL SHARD STATE
// let epoch = self.epoch_manager.current_epoch().await?;
//
//
//
// EXECUTE THE PAYLOAD AND GET THE RESULT (+ SUBSTATE CHANGES)
// fn process_payload(
// &self,
// payload: TPayload,
// pledges: HashMap<ShardId, Option<ObjectPledge>>,
// ) -> Result<FinalizeResult, PayloadProcessorError>;
// let finalize_result = self.payload_processor.process_payload(payload, shard_pledges)?;
//
//
