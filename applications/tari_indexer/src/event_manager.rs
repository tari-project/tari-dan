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

use std::{collections::HashMap, ops::RangeInclusive};

use tari_common::configuration::Network;
use futures::StreamExt;
use log::*;
use rand::{prelude::*, rngs::OsRng};
use tari_dan_common_types::{PeerAddress, SubstateAddress};
use tari_dan_p2p::proto::rpc::SyncBlocksRequest;
use tari_dan_storage::consensus_models::{Block, BlockId};
use tari_engine_types::{events::Event, substate::SubstateId};
use tari_epoch_manager::EpochManagerReader;
use tari_validator_node_rpc::client::{TariValidatorNodeRpcClientFactory, ValidatorNodeClientFactory};
use tari_dan_storage::consensus_models::TransactionRecord;
use tari_transaction::Transaction;

const LOG_TARGET: &str = "tari::indexer::event_manager";

pub struct EventManager {
    network: Network,
    epoch_manager: Box<dyn EpochManagerReader<Addr = PeerAddress>>,
    client_factory: TariValidatorNodeRpcClientFactory,
}

impl EventManager {
    pub fn new(
        network: Network,
        epoch_manager: Box<dyn EpochManagerReader<Addr = PeerAddress>>,
        client_factory: TariValidatorNodeRpcClientFactory,
    ) -> Self {
        Self {
            network,
            epoch_manager,
            client_factory,
        }
    }

    pub async fn scan_events(
        &self,
        start_block: Option<BlockId>,
        topic: Option<String>,
        substate_id: Option<SubstateId>,
    ) -> Result<Vec<Event>, anyhow::Error> {
        info!(
            target: LOG_TARGET,
            "scan_events: start_block={:?}, topic={:?}, substate_id={:?}",
            start_block,
            topic,
            substate_id
        );

        // get all the committees
        // TODO: optimize by getting all individual CommiteeShards instead of all the VNs
        let epoch = self.epoch_manager.current_epoch().await?;
        let full_range = RangeInclusive::new(SubstateAddress::zero(), SubstateAddress::max());
        let mut committee = self
            .epoch_manager
            .get_committee_within_shard_range(epoch, full_range)
            .await?;
        committee.members.shuffle(&mut OsRng);

        let mut blocks = HashMap::new();
        for member in committee.addresses() {
            let resp = self.get_blocks_from_vn(member).await;

            match resp {
                Ok(vn_blocks) => {
                    vn_blocks.into_iter().for_each(|b| {
                        blocks.insert(*b.id(), b);
                    });
                },
                Err(e) => {
                    warn!(
                        target: LOG_TARGET,
                        "Could not get blocks from vn {}: {}",
                        member,
                        e
                    );
                },
            };
        }

        info!(
            target: LOG_TARGET,
            "scan_events: blocks={:?}",
            blocks
        );

        Ok(vec![])
    }

    async fn get_blocks_from_vn(&self, vn_addr: &PeerAddress) -> Result<Vec<Block>, anyhow::Error> {
        let mut blocks = vec![];

        let mut rpc_client = self.client_factory.create_client(vn_addr);
        let mut client = rpc_client.client_connection().await?;

        // TODO: use the latest block id that we scanned
        let start_block = Block::zero_block(self.network);

        let mut stream = client
            .sync_blocks(SyncBlocksRequest {
                start_block_id: start_block.id().as_bytes().to_vec(),
            })
            .await?;
        while let Some(resp) = stream.next().await {
            let msg = resp?;
            info!(
                target: LOG_TARGET,
                "scan_events: msg={:?}",
                blocks
            );
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
            let num_substates = msg.substate_count().ok_or_else(|| {
                anyhow::anyhow!("Expected peer to return substate count")
            })? as usize;

            for _ in 0..num_substates {
                let Some(_) = stream.next().await else {
                    anyhow::bail!("Peer closed session before sending substate updates message")
                };
            }

            let Some(resp) = stream.next().await else {
                anyhow::bail!("Peer closed session before sending transactions message")
            };
            let msg = resp?;
            let transactions = msg.into_transactions().ok_or_else(|| anyhow::anyhow!("Expected peer to return transactions"))?;

            let transactions = transactions
                .into_iter()
                .map(Transaction::try_from)
                .map(|r| r.map(TransactionRecord::new))
                .collect::<Result<Vec<_>, _>>()?;

            info!(
                target: LOG_TARGET,
                "Synced block {} with {} transactions",
                block.id(),
                transactions.len(),
            );

            blocks.push(block);
        }

        Ok(blocks)
    }
}
