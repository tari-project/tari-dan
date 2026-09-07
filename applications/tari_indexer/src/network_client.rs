//    Copyright 2025 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{HashMap, HashSet},
    future::Future,
};

use indexmap::IndexMap;
use log::{info, warn};
use tari_epoch_manager::{EpochManagerError, EpochManagerReader};
use tari_ootle_common_types::{
    Epoch,
    NodeAddressable,
    NumPreshards,
    ShardGroup,
    SubstateAddress,
    displayable::Displayable,
    optional::Optional,
};
use tari_ootle_transaction::{Transaction, TransactionId};
use tari_rpc_framework::RpcStatusCode;
use tari_validator_node_rpc::{
    ValidatorNodeRpcClientError,
    client::{ValidatorNodeClientFactory, ValidatorNodeRpcClient},
};

const LOG_TARGET: &str = "tari::indexer::network_client";

#[derive(Debug, Clone)]
pub struct TariNetworkClient<TEpochManager, TClientFactory> {
    epoch_manager: TEpochManager,
    client_provider: TClientFactory,
    num_preshards: NumPreshards,
}

impl<TAddr, TEpochManager, TClientFactory> TariNetworkClient<TEpochManager, TClientFactory>
where
    TAddr: NodeAddressable + 'static,
    TEpochManager: EpochManagerReader<Addr = TAddr> + 'static,
    TClientFactory: ValidatorNodeClientFactory<TAddr> + 'static,
{
    pub fn new(epoch_manager: TEpochManager, client_provider: TClientFactory, num_preshards: NumPreshards) -> Self {
        Self {
            epoch_manager,
            client_provider,
            num_preshards,
        }
    }

    /// The current epoch, once the epoch manager has completed its initial scan. The scan is waited
    /// on because until it lands the epoch manager reports zero, and a caller deriving anything
    /// durable from a zero epoch — a retention key, a validity window — writes a value the node
    /// will disagree with seconds later.
    pub async fn current_epoch(&self) -> Result<Epoch, NetworkClientError> {
        self.epoch_manager.wait_for_initial_scanning_to_complete().await?;
        Ok(self.epoch_manager.current_epoch().await?)
    }

    pub async fn submit_transaction(&self, transaction: Transaction) -> Result<TransactionId, NetworkClientError> {
        // Ensure initial scanning has completed to ensure an accurate epoch
        self.epoch_manager.wait_for_initial_scanning_to_complete().await?;

        let tx_id = transaction.calculate_id();

        info!(
            target: LOG_TARGET,
            "Submitting transaction {} to the network", tx_id
        );

        let involved = transaction.involved_substate_addresses_iter().collect::<HashSet<_>>();

        let results = self
            .try_with_committee(involved, |mut client| {
                let transaction = transaction.clone();
                async move { client.submit_transaction(transaction).await }
            })
            .await?;

        let success_count = results.values().filter(|r| r.is_ok()).count();

        info!(
            target: LOG_TARGET,
            "Submitted transaction {} succeeded for {}/{} shard groups",
            tx_id,
            success_count,
            results.len()
        );
        if success_count != results.len() {
            warn!(
                target: LOG_TARGET,
                "Transaction {} was not submitted to some shard groups. {}",
                tx_id,
                results
                    .iter()
                    .filter_map(|(shard_group, result)| {
                        if let Err(err) = result {
                            Some(format!("Failed for {}: {}", shard_group, err))
                        } else {
                            None
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }

        Ok(tx_id)
    }

    pub async fn try_single_with_committee<'a, F, T, TFut>(
        &self,
        substate_address: SubstateAddress,
        callback: F,
    ) -> Result<T, NetworkClientError>
    where
        F: FnMut(TClientFactory::Client) -> TFut,
        TFut: Future<Output = Result<T, ValidatorNodeRpcClientError>> + 'a,
        TClientFactory::Client: 'a,
        T: 'static,
    {
        let results = self
            .try_with_committee(std::iter::once(substate_address), callback)
            .await?;

        results
            .into_values()
            .next()
            .expect("Expected exactly one result for single substate address")
            .map_err(|e| NetworkClientError::AllValidatorsFailed {
                committee_size: 1,
                last_error: Some(e),
            })
    }

    /// Fetches the committee members for the given shard and calls the given callback with each member until
    /// the callback returns a `Ok` with results for each shard group. If an Ok is returned, the hashmap is guaranteed
    /// to be the same size as the number of unique shard groups queried.
    pub async fn try_with_committee<'a, F, T, TFut, ISubstateAddr>(
        &self,
        substate_addresses: ISubstateAddr,
        mut callback: F,
    ) -> Result<IndexMap<ShardGroup, Result<T, ValidatorNodeRpcClientError>>, NetworkClientError>
    where
        F: FnMut(TClientFactory::Client) -> TFut,
        TFut: Future<Output = Result<T, ValidatorNodeRpcClientError>> + 'a,
        TClientFactory::Client: 'a,
        T: 'static,
        ISubstateAddr: IntoIterator<Item = SubstateAddress>,
    {
        let epoch = self.epoch_manager.current_epoch().await?;
        let num_committees = self.epoch_manager.get_num_committees(epoch).await?;

        info!(
            target: LOG_TARGET,
            "Fetching committee members at epoch {} ({} total committees)",
            epoch,
            num_committees,
        );

        let mut all_members = HashMap::new();
        for substate_address in substate_addresses {
            let shard_group = substate_address.to_shard_group(self.num_preshards, num_committees);

            if all_members.contains_key(&shard_group) {
                continue; // Already processed this shard group
            }

            // A shard group with no assigned validators is a statement about the network - nothing
            // answers for that part of the shard space at this epoch - rather than a failure of this
            // indexer, so it must reach the caller as an unavailable committee and not an internal
            // error.
            let committee = self
                .epoch_manager
                .get_committee_by_shard_group(epoch, shard_group)
                .await
                .optional()?
                .filter(|committee| !committee.is_empty())
                .ok_or(NetworkClientError::NoCommitteeForShardGroup { epoch, shard_group })?;
            all_members.insert(shard_group, committee);
        }

        let committee_size = all_members.len();
        if committee_size == 0 {
            return Err(NetworkClientError::NoCommitteeMembers);
        }

        let mut num_succeeded = 0;
        let mut results = IndexMap::with_capacity(committee_size);
        let mut last_error_sg = None;
        for (shard_group, committee) in all_members {
            for member in committee.shuffled() {
                let client = self.client_provider.create_client(&member.address);
                match callback(client).await {
                    Ok(ret) => {
                        num_succeeded += 1;
                        results.insert(shard_group, Ok(ret));
                        break; // Move onto the next shard group
                    },
                    Err(err) => {
                        warn!(
                            target: LOG_TARGET,
                            "Request failed for validator '{}': {}", member, err
                        );
                        last_error_sg = Some(shard_group);
                        results.insert(shard_group, Err(err));
                    },
                }
            }
        }

        if num_succeeded == 0 {
            let mut last_error = None;

            if let Some(sg) = last_error_sg {
                let last = results.swap_remove(&sg).expect("shard group must exist");
                match last {
                    Ok(_) => {},
                    Err(e) => last_error = Some(e),
                }
            }
            return Err(NetworkClientError::AllValidatorsFailed {
                committee_size,
                last_error,
            });
        }

        Ok(results)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NetworkClientError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Rpc call failed for all ({committee_size}) validators: {}", .last_error.display())]
    AllValidatorsFailed {
        committee_size: usize,
        last_error: Option<ValidatorNodeRpcClientError>,
    },
    #[error("No committee at present. Try again later")]
    NoCommitteeMembers,
    #[error("No validators are assigned to {shard_group} at {epoch}")]
    NoCommitteeForShardGroup { epoch: Epoch, shard_group: ShardGroup },
}

impl NetworkClientError {
    /// Returns the rejection details when every involved validator failed and the last failure was a
    /// BAD_REQUEST, i.e. the transaction was explicitly rejected as invalid (e.g. by mempool
    /// validation) rather than failing for transient reasons.
    pub fn validation_rejection_details(&self) -> Option<&str> {
        let NetworkClientError::AllValidatorsFailed {
            last_error: Some(rpc_err),
            ..
        } = self
        else {
            return None;
        };
        let status = rpc_err.status()?;
        if status.as_status_code() == RpcStatusCode::BadRequest {
            Some(status.details())
        } else {
            None
        }
    }
}
