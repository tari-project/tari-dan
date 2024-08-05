//  Copyright 2024 The Tari Project
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

use std::{collections::HashMap, str::FromStr};

use futures::StreamExt;
use log::*;
use tari_bor::decode;
use tari_common::configuration::Network;
use tari_crypto::tari_utilities::message_format::MessageFormat;
use tari_dan_app_utilities::consensus_constants::ConsensusConstants;
use tari_dan_common_types::{committee::Committee, Epoch, NumPreshards, PeerAddress, ShardGroup};
use tari_dan_p2p::proto::rpc::{GetTransactionResultRequest, PayloadResultStatus, SyncBlocksRequest};
use tari_dan_storage::consensus_models::{Block, BlockId, Decision, TransactionRecord};
use tari_engine_types::{
    commit_result::{ExecuteResult, TransactionResult},
    events::Event,
    substate::{Substate, SubstateId, SubstateValue},
};
use tari_epoch_manager::EpochManagerReader;
use tari_template_lib::models::{EntityId, TemplateAddress};
use tari_transaction::{Transaction, TransactionId};
use tari_validator_node_rpc::client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory};

use crate::{
    config::EventFilterConfig,
    event_data::EventData,
    substate_storage_sqlite::{
        models::{
            events::{NewEvent, NewScannedBlockId},
            substate::NewSubstate,
        },
        sqlite_substate_store_factory::{
            SqliteSubstateStore,
            SubstateStore,
            SubstateStoreReadTransaction,
            SubstateStoreWriteTransaction,
        },
    },
};

const LOG_TARGET: &str = "tari::indexer::event_scanner";

#[derive(Default, Debug, Clone)]
pub struct EventFilter {
    pub topic: Option<String>,
    pub entity_id: Option<EntityId>,
    pub substate_id: Option<SubstateId>,
    pub template_address: Option<TemplateAddress>,
}

impl TryFrom<EventFilterConfig> for EventFilter {
    type Error = anyhow::Error;

    fn try_from(cfg: EventFilterConfig) -> Result<Self, Self::Error> {
        let entity_id = cfg.entity_id.map(|str| EntityId::from_hex(&str)).transpose()?;
        let substate_id = cfg.substate_id.map(|str| SubstateId::from_str(&str)).transpose()?;
        let template_address = cfg
            .template_address
            .map(|str| TemplateAddress::from_str(&str))
            .transpose()?;

        Ok(Self {
            topic: cfg.topic,
            entity_id,
            substate_id,
            template_address,
        })
    }
}

#[derive(Default, Debug, Clone, PartialEq, Eq)]
struct TransactionMetadata {
    pub transaction_id: TransactionId,
    pub timestamp: u64,
}

pub struct EventScanner {
    network: Network,
    epoch_manager: Box<dyn EpochManagerReader<Addr = PeerAddress>>,
    client_factory: TariValidatorNodeRpcClientFactory,
    substate_store: SqliteSubstateStore,
    event_filters: Vec<EventFilter>,
}

impl EventScanner {
    pub fn new(
        network: Network,
        epoch_manager: Box<dyn EpochManagerReader<Addr = PeerAddress>>,
        client_factory: TariValidatorNodeRpcClientFactory,
        substate_store: SqliteSubstateStore,
        event_filters: Vec<EventFilter>,
    ) -> Self {
        Self {
            network,
            epoch_manager,
            client_factory,
            substate_store,
            event_filters,
        }
    }

    pub async fn scan_events(&self) -> Result<usize, anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "scan_events",
        );

        let mut event_count = 0;

        let current_epoch = self.epoch_manager.current_epoch().await?;
        let current_committees = self.epoch_manager.get_committees(current_epoch).await?;
        for (shard_group, mut committee) in current_committees {
            info!(
                target: LOG_TARGET,
                "Scanning committee epoch={}, sg={}",
                current_epoch,
                shard_group
            );
            let new_blocks = self
                .get_new_blocks_from_committee(shard_group, &mut committee, current_epoch)
                .await?;
            info!(
                target: LOG_TARGET,
                "Scanned {} blocks",
                new_blocks.len()
            );
            let transactions = self.extract_transactions_from_blocks(new_blocks);
            info!(
                target: LOG_TARGET,
                "Scanned {} transactions",
                transactions.len()
            );

            for transaction in transactions {
                // fetch all the events in the transaction
                let events = self.get_events_for_transaction(transaction.transaction_id).await?;
                event_count += events.len();

                // only keep the events specified by the indexer filter
                let filtered_events: Vec<EventData> =
                    events.into_iter().filter(|ev| self.should_persist_event(ev)).collect();
                info!(
                    target: LOG_TARGET,
                    "Filtered events: {}",
                    filtered_events.len()
                );
                self.store_events_in_db(&filtered_events, transaction).await?;
            }
        }

        info!(
            target: LOG_TARGET,
            "Scanned {} events",
            event_count
        );

        Ok(event_count)
    }

    fn should_persist_event(&self, event_data: &EventData) -> bool {
        for filter in &self.event_filters {
            if Self::event_matches_filter(filter, &event_data.event) {
                return true;
            }
        }

        false
    }

    fn event_matches_filter(filter: &EventFilter, event: &Event) -> bool {
        let matches_topic = filter.topic.as_ref().map_or(true, |t| *t == event.topic());
        let matches_template = filter
            .template_address
            .as_ref()
            .map_or(true, |t| *t == event.template_address());

        let matches_substate_id = match &filter.substate_id {
            Some(substate_id) => event.substate_id().map(|s| s == *substate_id).unwrap_or(false),
            None => true,
        };

        let matches_entity_id = match &filter.entity_id {
            Some(entity_id) => event
                .substate_id()
                .map(|s| Self::entity_id_matches(&s, entity_id))
                .unwrap_or(false),
            None => true,
        };

        if matches_topic && matches_template && matches_substate_id && matches_entity_id {
            return true;
        }

        false
    }

    fn entity_id_matches(substate_id: &SubstateId, entity_id: &EntityId) -> bool {
        match substate_id {
            SubstateId::Component(c) => c.entity_id() == *entity_id,
            SubstateId::Resource(r) => r.as_entity_id() == *entity_id,
            SubstateId::Vault(v) => v.entity_id() == *entity_id,
            // TODO: should all types of substate addresses expose the entity id?
            _ => false,
        }
    }

    async fn store_events_in_db(
        &self,
        events_data: &Vec<EventData>,
        transaction: TransactionMetadata,
    ) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx()?;

        for data in events_data {
            let event_row = NewEvent {
                template_address: data.event.template_address().to_string(),
                tx_hash: data.event.tx_hash().to_string(),
                topic: data.event.topic(),
                payload: data.event.payload().to_json().expect("Failed to convert to JSON"),
                substate_id: data.event.substate_id().map(|s| s.to_string()),
                version: 0_i32,
                timestamp: transaction.timestamp as i64,
            };

            // TODO: properly avoid or handle duplicated events
            //       For now we will just check if a similar event exists in the db
            let event_already_exists = tx.event_exists(event_row.clone())?;
            if event_already_exists {
                // the event was already stored previously
                warn!(
                    target: LOG_TARGET,
                    "Duplicated event {:}",
                    data.event
                );
                continue;
            }

            info!(
                target: LOG_TARGET,
                "Saving event: {:?}",
                event_row
            );
            tx.save_event(event_row)?;

            // store/update the related substate if any
            if let (Some(substate_id), Some(substate)) = (data.event.substate_id(), &data.substate) {
                let template_address = Self::extract_template_address_from_substate(substate).map(|t| t.to_string());
                let module_name = Self::extract_module_name_from_substate(substate);
                let substate_row = NewSubstate {
                    address: substate_id.to_string(),
                    version: i64::from(substate.version()),
                    data: Self::encode_substate(substate)?,
                    tx_hash: data.event.tx_hash().to_string(),
                    template_address,
                    module_name,
                    timestamp: transaction.timestamp as i64,
                };
                debug!(
                    target: LOG_TARGET,
                    "Saving substate: {:?}",
                    substate_row
                );
                tx.set_substate(substate_row)?;
            }
        }

        tx.commit()?;

        Ok(())
    }

    fn extract_template_address_from_substate(substate: &Substate) -> Option<TemplateAddress> {
        match substate.substate_value() {
            SubstateValue::Component(c) => Some(c.template_address),
            _ => None,
        }
    }

    fn extract_module_name_from_substate(substate: &Substate) -> Option<String> {
        match substate.substate_value() {
            SubstateValue::Component(c) => Some(c.module_name.to_owned()),
            _ => None,
        }
    }

    fn encode_substate(substate: &Substate) -> Result<String, anyhow::Error> {
        let pretty_json = serde_json::to_string_pretty(&substate)?;
        Ok(pretty_json)
    }

    async fn get_events_for_transaction(&self, transaction_id: TransactionId) -> Result<Vec<EventData>, anyhow::Error> {
        let committee = self.get_all_vns().await?;

        for member in &committee {
            let resp = self.get_execute_result_from_vn(member, &transaction_id).await;

            match resp {
                Ok(res) => {
                    if let Some(execute_result) = res {
                        let events = self.extract_events_from_transaction_result(execute_result);
                        return Ok(events);
                    } else {
                        // The transaction is not successful, so we don't return any events
                        return Ok(vec![]);
                    }
                },
                Err(e) => {
                    // We do nothing on a single VN failure, we only log it
                    warn!(
                        target: LOG_TARGET,
                        "Could not get transaction result from vn {}: {}",
                        member,
                        e
                    );
                },
            };
        }

        warn!(
            target: LOG_TARGET,
            "We could not get transaction results from any of the vns",
        );
        Ok(vec![])
    }

    async fn get_execute_result_from_vn(
        &self,
        vn_addr: &PeerAddress,
        transaction_id: &TransactionId,
    ) -> Result<Option<ExecuteResult>, anyhow::Error> {
        let mut rpc_client = self.client_factory.create_client(vn_addr);
        let mut client = rpc_client.client_connection().await?;

        let response = client
            .get_transaction_result(GetTransactionResultRequest {
                transaction_id: transaction_id.as_bytes().to_vec(),
            })
            .await?;

        match PayloadResultStatus::try_from(response.status) {
            Ok(PayloadResultStatus::Finalized) => {
                let proto_decision = tari_dan_p2p::proto::consensus::Decision::try_from(response.final_decision)?;
                let final_decision = proto_decision.try_into()?;
                if let Decision::Commit = final_decision {
                    Ok(Some(response.execution_result)
                        .filter(|r| !r.is_empty())
                        .map(|r| decode(&r))
                        .transpose()?)
                } else {
                    Ok(None)
                }
            },
            _ => Ok(None),
        }
    }

    fn extract_events_from_transaction_result(&self, result: ExecuteResult) -> Vec<EventData> {
        if let TransactionResult::Accept(substate_diff) = result.finalize.result {
            let substates: HashMap<SubstateId, Substate> = substate_diff.into_up_iter().collect();

            let events = result
                .finalize
                .events
                .into_iter()
                .map(|event| {
                    let substate = if let Some(substate_id) = event.substate_id() {
                        substates.get(&substate_id).cloned()
                    } else {
                        None
                    };

                    EventData { event, substate }
                })
                .collect();

            events
        } else {
            vec![]
        }
    }

    fn extract_transactions_from_blocks(&self, blocks: Vec<Block>) -> Vec<TransactionMetadata> {
        blocks
            .iter()
            .flat_map(|b| b.all_accepted_transactions_ids().map(|id| (id, b.timestamp())))
            .map(|(transaction_id, timestamp)| TransactionMetadata {
                transaction_id: *transaction_id,
                timestamp,
            })
            .collect()
    }

    fn build_genesis_block_id(&self, num_preshards: NumPreshards) -> BlockId {
        // TODO: this should return the actual genesis for the shard group and epoch
        let start_block = Block::zero_block(self.network, num_preshards);
        *start_block.id()
    }

    #[allow(unused_assignments)]
    async fn get_new_blocks_from_committee(
        &self,
        shard_group: ShardGroup,
        committee: &mut Committee<PeerAddress>,
        epoch: Epoch,
    ) -> Result<Vec<Block>, anyhow::Error> {
        // We start scanning from the last scanned block for this commitee
        let start_block_id = self
            .substate_store
            .with_read_tx(|tx| tx.get_last_scanned_block_id(epoch, shard_group))?;
        let start_block_id = start_block_id.unwrap_or_else(|| {
            let consensus_constants = ConsensusConstants::from(self.network);
            self.build_genesis_block_id(consensus_constants.num_preshards)
        });

        committee.shuffle();
        let mut last_block_id = start_block_id;

        info!(
            target: LOG_TARGET,
            "Scanning new blocks since {} from (epoch={}, shard={})",
            last_block_id,
            epoch,
            shard_group
        );

        for member in committee.members() {
            debug!(
                target: LOG_TARGET,
                "Trying to get blocks from VN {} (epoch={}, shard_group={})",
                member,
                epoch,
                shard_group
            );
            let resp = self.get_blocks_from_vn(member, start_block_id).await;

            match resp {
                Ok(blocks) => {
                    // TODO: try more than 1 VN per commitee
                    info!(
                        target: LOG_TARGET,
                        "Got {} blocks from VN {} (epoch={}, shard_group={})",
                        blocks.len(),
                        member,
                        epoch,
                        shard_group,
                    );
                    if let Some(block) = blocks.last() {
                        last_block_id = *block.id();
                    }
                    // Store the latest scanned block id in the database for future scans
                    self.save_scanned_block_id(epoch, shard_group, last_block_id)?;
                    return Ok(blocks);
                },
                Err(e) => {
                    // We do nothing on a single VN failure, we only log it
                    warn!(
                        target: LOG_TARGET,
                        "Could not get blocks from VN {} (epoch={}, shard_group={}): {}",
                        member,
                        epoch,
                        shard_group,
                        e
                    );
                },
            };
        }

        // We don't raise an error if none of the VNs have blocks, the scanning will retry eventually
        warn!(
            target: LOG_TARGET,
            "Could not get blocks from any of the VNs of the committee (epoch={}, shard_group={})",
            epoch,
            shard_group
        );
        Ok(vec![])
    }

    fn save_scanned_block_id(
        &self,
        epoch: Epoch,
        shard_group: ShardGroup,
        last_block_id: BlockId,
    ) -> Result<(), anyhow::Error> {
        let row = NewScannedBlockId {
            epoch: epoch.0 as i64,
            shard_group: shard_group.encode_as_u32() as i32,
            last_block_id: last_block_id.as_bytes().to_vec(),
        };
        self.substate_store.with_write_tx(|tx| tx.save_scanned_block_id(row))?;
        Ok(())
    }

    async fn get_all_vns(&self) -> Result<Vec<PeerAddress>, anyhow::Error> {
        // get all the committees
        let epoch = self.epoch_manager.current_epoch().await?;
        Ok(self
            .epoch_manager
            .get_all_validator_nodes(epoch)
            .await
            .map(|v| v.iter().map(|m| m.address).collect())?)
    }

    async fn get_blocks_from_vn(
        &self,
        vn_addr: &PeerAddress,
        start_block_id: BlockId,
    ) -> Result<Vec<Block>, anyhow::Error> {
        let mut blocks = vec![];

        let mut rpc_client = self.client_factory.create_client(vn_addr);
        let mut client = rpc_client.client_connection().await?;

        let mut stream = client
            .sync_blocks(SyncBlocksRequest {
                start_block_id: start_block_id.as_bytes().to_vec(),
                up_to_epoch: None,
            })
            .await?;
        while let Some(resp) = stream.next().await {
            let msg = resp?;

            let new_block = msg
                .into_block()
                .ok_or_else(|| anyhow::anyhow!("Expected peer to return a newblock"))?;
            let block = Block::try_from(new_block)?;

            let Some(_) = stream.next().await else {
                anyhow::bail!("Peer closed session before sending QC message")
            };

            let Some(resp) = stream.next().await else {
                anyhow::bail!("Peer closed session before sending substate update count message")
            };
            let msg = resp?;
            let num_substates =
                msg.substate_count()
                    .ok_or_else(|| anyhow::anyhow!("Expected peer to return substate count"))? as usize;

            for _ in 0..num_substates {
                let Some(_) = stream.next().await else {
                    anyhow::bail!("Peer closed session before sending substate updates message")
                };
            }

            let Some(resp) = stream.next().await else {
                anyhow::bail!("Peer closed session before sending transactions message")
            };
            let msg = resp?;
            let transactions = msg
                .into_transactions()
                .ok_or_else(|| anyhow::anyhow!("Expected peer to return transactions"))?;

            let _transactions = transactions
                .into_iter()
                .map(Transaction::try_from)
                .map(|r| r.map(TransactionRecord::new))
                .collect::<Result<Vec<_>, _>>()?;

            blocks.push(block);
        }

        Ok(blocks)
    }
}
