use tari_dan_engine::transaction::Transaction;
use tari_dan_storage_sqlite::sqlite_shard_store_factory::SqliteShardStoreFactory;
use tari_engine_types::commit_result::FinalizeResult;

use crate::{
    p2p::services::{epoch_manager::handle::EpochManagerHandle, template_manager::TemplateManager},
    payload_processor::TariDanPayloadProcessor,
};

#[derive(Clone)]
pub struct DryRunTransactionProcessor {
    /// The epoch manager
    _epoch_manager: EpochManagerHandle,
    /// The payload processor. This determines whether a payload proposal results in an accepted or rejected vote.
    _payload_processor: TariDanPayloadProcessor<TemplateManager>,
    /// Store used to persist consensus state.
    _shard_store: SqliteShardStoreFactory,
}

impl DryRunTransactionProcessor {
    pub fn new(
        _epoch_manager: EpochManagerHandle,
        _payload_processor: TariDanPayloadProcessor<TemplateManager>,
        _shard_store: SqliteShardStoreFactory,
    ) -> Self {
        Self {
            _epoch_manager,
            _payload_processor,
            _shard_store,
        }
    }

    // TODO: create a error enum
    pub async fn process_transaction(&self, _transaction: Transaction) -> Result<FinalizeResult, String> {
        dbg!(&self._epoch_manager);
        dbg!(&self._payload_processor);
        dbg!(&self._shard_store);
        Err("TODO".to_owned())
    }
}

// BUILD THE PAYLOAD
// let payload = TariDanPayload::new(transaction.clone());
//
// GET THE LIST OF INVOLVED SHARDS
// fn involved_shards(&self) -> Vec<ShardId>;
// payload.invoved_shards
//
//
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
