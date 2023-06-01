//  Copyright 2023, The Tari Project
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

use log::*;
use rand::{prelude::*, rngs::OsRng};
use tari_dan_common_types::{NodeAddressable, ShardId};
use tari_engine_types::{
    events::Event,
    substate::{SubstateAddress, SubstateValue},
};
use tari_epoch_manager::{base_layer::EpochManagerError, EpochManager};
use tari_template_lib::{
    models::NonFungibleIndexAddress,
    prelude::{ComponentAddress, ResourceAddress},
    Hash,
};
use tari_validator_node_rpc::client::{SubstateResult, ValidatorNodeClientFactory, ValidatorNodeRpcClient};

use crate::{error::IndexerError, NonFungibleSubstate};

const LOG_TARGET: &str = "tari::indexer::dan_layer_scanner";

pub struct SubstateScanner<TEpochManager, TVnClient> {
    committee_provider: TEpochManager,
    validator_node_client_factory: TVnClient,
}

impl<TEpochManager, TVnClient, TAddr> SubstateScanner<TEpochManager, TVnClient>
where
    TAddr: NodeAddressable,
    TEpochManager: EpochManager<TAddr, Error = EpochManagerError>,
    TVnClient: ValidatorNodeClientFactory<Addr = TAddr>,
{
    pub fn new(committee_provider: TEpochManager, validator_node_client_factory: TVnClient) -> Self {
        Self {
            committee_provider,
            validator_node_client_factory,
        }
    }

    pub async fn get_non_fungibles(
        &self,
        resource_address: &ResourceAddress,
        start_index: u64,
        end_index: Option<u64>,
    ) -> Result<Vec<NonFungibleSubstate>, IndexerError> {
        let mut nft_substates = vec![];
        let mut index = start_index;

        loop {
            // build the address of the nft index substate
            let index_address = NonFungibleIndexAddress::new(*resource_address, index);
            let index_substate_address = SubstateAddress::NonFungibleIndex(index_address);

            // get the nft index substate from the network
            // nft index substates are immutable, so they are always on version 0
            let index_substate_result = self
                .get_specific_substate_from_committee(&index_substate_address, 0)
                .await?;
            let index_substate = match index_substate_result {
                SubstateResult::Up { substate, .. } => substate.into_substate_value(),
                _ => break,
            };

            // now that we have the index substate, we need the latest substate of the referenced nft
            let nft_address = match index_substate.into_non_fungible_index() {
                Some(idx) => idx.referenced_address().clone(),
                // the protocol should never produce this scenario, we stop querying for more indexes if it happens
                None => break,
            };
            let nft_substate_address = SubstateAddress::NonFungible(nft_address);
            let SubstateResult::Up{substate, ..} = self.get_latest_substate_from_committee(&nft_substate_address, 0).await? else {
                break;
            };

            nft_substates.push(NonFungibleSubstate {
                index,
                address: nft_substate_address,
                substate,
            });

            if let Some(end_index) = end_index {
                if index >= end_index {
                    break;
                }
            }

            index += 1;
        }

        Ok(nft_substates)
    }

    /// Attempts to find the latest substate for the given address. If the lowest possible version is known, it can be
    /// provided to reduce effort/time required to scan.
    pub async fn get_substate(
        &self,
        substate_address: &SubstateAddress,
        version_hint: Option<u32>,
    ) -> Result<SubstateResult, IndexerError> {
        info!(target: LOG_TARGET, "get_substate: {} ", substate_address);

        self.get_latest_substate_from_committee(substate_address, version_hint.unwrap_or(0))
            .await
    }

    async fn get_latest_substate_from_committee(
        &self,
        substate_address: &SubstateAddress,
        lowest_version: u32,
    ) -> Result<SubstateResult, IndexerError> {
        // we keep asking from version 0 upwards
        let mut version = lowest_version;
        let mut last_result = None;
        loop {
            let substate_result = self
                .get_specific_substate_from_committee(substate_address, version)
                .await?;
            match substate_result {
                // when it's a "Down" state, we need to ask a higher version until we find an "Up" or "DoesNotExist"
                result @ SubstateResult::Down { .. } => {
                    last_result = Some(result);
                    version += 1;
                },
                SubstateResult::DoesNotExist => {
                    return Ok(last_result.unwrap_or(SubstateResult::DoesNotExist));
                },
                _ => return Ok(substate_result),
            }
        }
    }

    /// Returns a specific version. If this is not found an error is returned.
    pub async fn get_specific_substate_from_committee(
        &self,
        substate_address: &SubstateAddress,
        version: u32,
    ) -> Result<SubstateResult, IndexerError> {
        let shard = ShardId::from_address(substate_address, version);
        let epoch = self.committee_provider.current_epoch().await?;
        let mut committee = self.committee_provider.get_committee(epoch, shard).await?;

        committee.members.shuffle(&mut OsRng);
        let f = (committee.members.len() - 1) / 3;
        let mut num_down_substate_results = 0;
        for vn_public_key in &committee.members {
            match self
                .get_substate_from_vn(vn_public_key, substate_address, version)
                .await
            {
                Ok(substate_result) => match substate_result {
                    SubstateResult::Up { .. } | SubstateResult::Down { .. } => return Ok(substate_result),
                    SubstateResult::DoesNotExist => {
                        if num_down_substate_results >= f + 1 {
                            return Ok(substate_result);
                        }
                        num_down_substate_results += 1;
                    },
                },
                Err(e) => {
                    // We ignore a single VN error and keep querying the rest of the committee
                    error!(
                        target: LOG_TARGET,
                        "Could not get substate {}:{} from vn {}: {}", substate_address, version, vn_public_key, e
                    );
                },
            }
        }

        error!(
            target: LOG_TARGET,
            "Could not get substate {}:{} from any of the validator nodes", substate_address, version,
        );

        Ok(SubstateResult::DoesNotExist)
    }

    /// Gets a substate directly from querying a VN
    async fn get_substate_from_vn(
        &self,
        vn_public_key: &TAddr,
        address: &SubstateAddress,
        version: u32,
    ) -> Result<SubstateResult, IndexerError> {
        // build a client with the VN
        let mut client = self.validator_node_client_factory.create_client(vn_public_key);
        let result = client
            .get_substate(address, version)
            .await
            .map_err(|e| IndexerError::ValidatorNodeClientError(e.to_string()))?;
        Ok(result)
    }

    /// Queries the network to obtain events emitted in a single transaction
    pub async fn get_events_for_transaction(&self, transaction_hash: Hash) -> Result<Vec<Event>, IndexerError> {
        let substate_address = SubstateAddress::TransactionReceipt(transaction_hash.into());
        let substate = self.get_specific_substate_from_committee(&substate_address, 0).await?;
        let substate_value = if let SubstateResult::Up { substate, .. } = substate {
            substate.substate_value().clone()
        } else {
            return Err(IndexerError::InvalidSubstateState);
        };
        let events = if let SubstateValue::TransactionReceipt(tx_receipt) = substate_value {
            tx_receipt.events
        } else {
            return Err(IndexerError::InvalidSubstateValue);
        };

        Ok(events)
    }

    /// Queries the network to obtain a transaction hash from a given substate address and version
    async fn get_transaction_hash_from_substate_address(
        &self,
        substate_address: &SubstateAddress,
        version: u32,
    ) -> Result<Hash, IndexerError> {
        let shard_id = ShardId::from_address(substate_address, version);

        let epoch = self.committee_provider.current_epoch().await?;
        let mut committee = self.committee_provider.get_committee(epoch, shard_id).await?;

        committee.members.shuffle(&mut OsRng);

        let mut transaction_hash = None;
        for member in &committee.members {
            match self.get_substate_from_vn(member, substate_address, version).await {
                Ok(substate_result) => match substate_result {
                    SubstateResult::Up {
                        created_by_tx: tx_hash, ..
                    } |
                    SubstateResult::Down {
                        created_by_tx: tx_hash, ..
                    } => {
                        transaction_hash = Some(tx_hash);
                        break;
                    },
                    SubstateResult::DoesNotExist => {
                        warn!(
                            target: LOG_TARGET,
                            "validator node: {} does not have state for component_address = {} and version = {}",
                            member,
                            substate_address.as_component_address().unwrap(),
                            version
                        );
                        continue;
                    },
                },
                Err(e) => {
                    warn!(
                        target: LOG_TARGET,
                        "Could not find substate result for component_address = {} and version = {}, with error = {}",
                        substate_address.as_component_address().unwrap(),
                        version,
                        e
                    );
                    continue;
                },
            }
        }

        if let Some(tx_hash) = transaction_hash {
            Hash::try_from(tx_hash.as_slice()).map_err(|e| IndexerError::FailedToParseTransactionHash(e.to_string()))
        } else {
            // no transaction was found in the network for this component address and version
            Err(IndexerError::NotFoundTransaction(substate_address.clone(), version))
        }
    }

    /// Queries the network to obtain all the events associated with a component and
    /// a specific version.
    pub async fn get_events_for_component_and_version(
        &self,
        component_address: ComponentAddress,
        version: u32,
    ) -> Result<Vec<Event>, IndexerError> {
        let substate_address = SubstateAddress::Component(component_address);

        let tx_hash = self
            .get_transaction_hash_from_substate_address(&substate_address, version)
            .await?;

        match self.get_events_for_transaction(tx_hash).await {
            Ok(tx_events) => {
                // we need to filter all transaction events, by those corresponding
                // to the current component address
                let component_tx_events = tx_events
                    .into_iter()
                    .filter(|e| e.component_address().is_some() && e.component_address().unwrap() == component_address)
                    .collect::<Vec<Event>>();
                Ok(component_tx_events)
            },
            Err(e) => Err(e),
        }
    }

    /// Queries the network to obtain all the events associated with a component,
    /// starting at an optional version (if `None`, starts from `0`).
    pub async fn get_events_for_component(
        &self,
        component_address: ComponentAddress,
        version: Option<u32>,
    ) -> Result<Vec<(u32, Event)>, IndexerError> {
        let mut events = vec![];
        let mut version: u32 = version.unwrap_or_default();

        loop {
            match self
                .get_events_for_component_and_version(component_address, version)
                .await
            {
                Ok(component_tx_events) => events.extend(
                    component_tx_events
                        .into_iter()
                        .map(|e| (version, e))
                        .collect::<Vec<_>>(),
                ),
                Err(IndexerError::NotFoundTransaction(..)) => return Ok(events),
                Err(e) => return Err(e),
            }

            version += 1;
        }
    }
}
