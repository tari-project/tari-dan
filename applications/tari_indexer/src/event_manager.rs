//  Copyright 20234 The Tari Project
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

use std::{
    collections::{BTreeMap, HashSet},
    ops::RangeInclusive,
    str::FromStr,
};

use futures::StreamExt;
use log::*;
use rand::{prelude::*, rngs::OsRng};
use tari_bor::decode;
use tari_common::configuration::Network;
use tari_crypto::tari_utilities::message_format::MessageFormat;
use tari_dan_common_types::{committee::Committee, PeerAddress, SubstateAddress};
use tari_dan_p2p::proto::rpc::{GetTransactionResultRequest, PayloadResultStatus, SyncBlocksRequest};
use tari_dan_storage::consensus_models::{Block, BlockId, Command, Decision, TransactionRecord};
use tari_engine_types::{commit_result::ExecuteResult, events::Event, substate::SubstateId};
use tari_epoch_manager::EpochManagerReader;
use tari_template_lib::{models::Metadata, Hash};
use tari_transaction::{Transaction, TransactionId};
use tari_validator_node_rpc::client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory};

use crate::substate_storage_sqlite::{
    models::events::NewEvent,
    sqlite_substate_store_factory::{
        SqliteSubstateStore,
        SubstateStore,
        SubstateStoreReadTransaction,
        SubstateStoreWriteTransaction,
    },
};

const LOG_TARGET: &str = "tari::indexer::event_manager";

pub struct EventManager {
    network: Network,
    epoch_manager: Box<dyn EpochManagerReader<Addr = PeerAddress>>,
    client_factory: TariValidatorNodeRpcClientFactory,
    substate_store: SqliteSubstateStore,
}

impl EventManager {
    pub fn new(
        network: Network,
        epoch_manager: Box<dyn EpochManagerReader<Addr = PeerAddress>>,
        client_factory: TariValidatorNodeRpcClientFactory,
        substate_store: SqliteSubstateStore,
    ) -> Self {
        Self {
            network,
            epoch_manager,
            client_factory,
            substate_store,
        }
    }

    pub async fn find_events_in_db(
        &self,
        topic: Option<String>,
        substate_id: Option<SubstateId>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Event>, anyhow::Error> {
        // TODO: scanning should be done in a background process, not here
        // TODO: AND use the latest block id that we scanned
        self.scan_events(None).await?;

        let events = self.get_events_from_db(topic, substate_id, offset, limit).await?;
        Ok(events)
    }

    pub async fn scan_events(&self, start_block: Option<BlockId>) -> Result<(), anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "scan_events: start_block={:?}",
            start_block,
        );

        let new_blocks = self.get_new_blocks(start_block).await?;
        let transaction_ids = self.extract_transaction_ids_from_blocks(new_blocks);

        let mut events = vec![];
        for transaction_id in transaction_ids {
            let mut transaction_events = self.get_events_for_transaction(transaction_id).await?;
            events.append(&mut transaction_events);
        }

        self.store_events_in_db(&events).await?;

        info!(
        target: LOG_TARGET,
            "Scanned {} events",
            events.len()
        );

        Ok(())
    }

    async fn get_events_from_db(
        &self,
        topic: Option<String>,
        substate_id: Option<SubstateId>,
        offset: u32,
        limit: u32,
    ) -> Result<Vec<Event>, anyhow::Error> {
        let mut tx = self.substate_store.create_read_tx()?;
        let rows = tx.get_events(substate_id, topic, offset, limit)?;

        let mut events = vec![];
        for row in rows {
            let substate_id = row.substate_id.map(|str| SubstateId::from_str(&str)).transpose()?;
            let template_address = Hash::from_hex(&row.template_address)?;
            let tx_hash = Hash::from_hex(&row.tx_hash)?;
            let topic = row.topic;
            let payload = Metadata::from(serde_json::from_str::<BTreeMap<String, String>>(row.payload.as_str())?);
            events.push(Event::new(substate_id, template_address, tx_hash, topic, payload));
        }

        Ok(events)
    }

    async fn store_events_in_db(&self, events: &Vec<Event>) -> Result<(), anyhow::Error> {
        let mut tx = self.substate_store.create_write_tx()?;

        for event in events {
            let row = NewEvent {
                template_address: event.template_address().to_string(),
                tx_hash: event.tx_hash().to_string(),
                topic: event.topic(),
                payload: event.payload().to_json().expect("Failed to convert to JSON"),
                substate_id: event.substate_id().map(|s| s.to_string()),
                version: 0_i32,
            };
            tx.save_event(row)?;
        }

        tx.commit()?;

        Ok(())
    }

    async fn get_events_for_transaction(&self, transaction_id: TransactionId) -> Result<Vec<Event>, anyhow::Error> {
        let committee = self.get_all_vns().await?;

        for member in committee.addresses() {
            let resp = self.get_execute_result_from_vn(member, &transaction_id).await;

            match resp {
                Ok(res) => {
                    if let Some(execute_result) = res {
                        return Ok(execute_result.finalize.events);
                    } else {
                        // The transaction is not successful, we don't return any events
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
            "We could not get transaction result from any of the vns",
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

    fn extract_transaction_ids_from_blocks(&self, blocks: Vec<Block>) -> HashSet<TransactionId> {
        let mut transaction_ids = HashSet::new();

        for block in blocks {
            for command in block.commands() {
                match command {
                    Command::Accept(t) => {
                        transaction_ids.insert(*t.id());
                    },
                    _ => {
                        // we are only interested in confirmed transactions
                    },
                }
            }
        }

        transaction_ids
    }

    async fn get_new_blocks(&self, start_block: Option<BlockId>) -> Result<Vec<Block>, anyhow::Error> {
        let mut blocks = vec![];

        let committee = self.get_all_vns().await?;

        let start_block_id = match start_block {
            Some(id) => id,
            None => {
                let start_block = Block::zero_block(self.network);
                *start_block.id()
            },
        };

        for member in committee.addresses() {
            let resp = self.get_blocks_from_vn(member, start_block_id).await;

            match resp {
                Ok(mut vn_blocks) => {
                    blocks.append(&mut vn_blocks);
                },
                Err(e) => {
                    // We do nothing on a single VN failure, we only log it
                    warn!(
                        target: LOG_TARGET,
                        "Could not get blocks from vn {}: {}",
                        member,
                        e
                    );
                },
            };
        }

        Ok(blocks)
    }

    async fn get_all_vns(&self) -> Result<Committee<PeerAddress>, anyhow::Error> {
        // get all the committees
        // TODO: optimize by getting all individual CommiteeShards instead of all the VNs
        let epoch = self.epoch_manager.current_epoch().await?;
        let full_range = RangeInclusive::new(SubstateAddress::zero(), SubstateAddress::max());
        let mut committee = self
            .epoch_manager
            .get_committee_within_shard_range(epoch, full_range)
            .await?;
        committee.members.shuffle(&mut OsRng);

        Ok(committee)
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
            })
            .await?;
        while let Some(resp) = stream.next().await {
            let msg = resp?;

            let new_block = msg
                .into_block()
                .ok_or_else(|| anyhow::anyhow!("Expected peer to return a newblock"))?;
            let block = Block::try_from(new_block)?;
            info!(
                target: LOG_TARGET,
                "scan_events: block={:?}",
                block
            );

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
