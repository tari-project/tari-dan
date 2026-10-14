//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashSet,
    fmt::Display,
    future::Future,
    ops::{Deref, DerefMut},
};

use log::*;
use tari_epoch_manager::{EpochManagerError, EpochManagerReader, service::EpochManagerHandle};
use tari_networking::NetworkingHandle;
use tari_ootle_common_types::{Epoch, ShardGroup, optional::Optional};
use tari_ootle_p2p::{PeerAddress, TariMessagingSpec, ToPeerId};
use tari_validator_node_rpc::{ValidatorNodeRpcClientError, client::RpcMultiPool, rpc_service};

const LOG_TARGET: &str = "tari::indexer::network_state_sync::committee_client";

#[derive(Debug, Clone)]
pub struct ValidatorCommitteeRpcPool {
    shard_group: ShardGroup,
    pool: RpcMultiPool<TariMessagingSpec>,
    epoch_manager: EpochManagerHandle<PeerAddress>,
    failed_peers: FailedPeers,
}

impl ValidatorCommitteeRpcPool {
    pub fn new(
        shard_group: ShardGroup,
        networking: NetworkingHandle<TariMessagingSpec>,
        epoch_manager: EpochManagerHandle<PeerAddress>,
    ) -> Self {
        Self {
            shard_group,
            pool: RpcMultiPool::new(networking),
            epoch_manager,
            failed_peers: FailedPeers::default(),
        }
    }

    /// A session with a random committee member, shared with every other caller of this pool that reaches the same
    /// member. A session serves one request at a time, so a stream that stays open belongs on
    /// [`Self::new_stream_session`].
    pub async fn new_session(&mut self) -> Result<ValidatorRpcSession, ValidatorCommitteeClientError> {
        self.random_member_session(SessionKind::Pooled).await
    }

    /// A session with a random committee member that nothing else uses, for a stream that stays open for as long as
    /// the member has more to send. Dropping it closes it, which ends the stream on the member's side.
    pub async fn new_stream_session(&mut self) -> Result<ValidatorRpcSession, ValidatorCommitteeClientError> {
        self.random_member_session(SessionKind::Dedicated).await
    }

    async fn random_member_session(
        &mut self,
        kind: SessionKind,
    ) -> Result<ValidatorRpcSession, ValidatorCommitteeClientError> {
        let epoch = self.epoch_manager.get_current_epoch();
        let mut last_error = None::<ValidatorCommitteeClientError>;
        loop {
            let member = self
                .epoch_manager
                .get_random_committee_member(epoch, Some(self.shard_group), self.failed_peers.at(epoch).clone())
                .await
                .optional()?;

            let Some(member) = member else {
                // All validators have been attempted and failed - no real choice but to clear the past failed nodes and
                // try again if this is called again
                let committee_size = self.failed_peers.at(epoch).len();
                self.failed_peers.clear();
                return Err(ValidatorCommitteeClientError::AllValidatorsFailed {
                    committee_size,
                    last_error: last_error.as_ref().map(|e| e.to_string()),
                });
            };
            let result = self.session_for_peer(member.address, kind).await;
            match result {
                Ok(session) => return Ok(session),
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to create new session for validator '{}': {}", member, err
                    );
                    last_error = Some(err);
                    self.failed_peers.at(epoch).insert(member.address);
                },
            }
        }
    }

    async fn session_for_peer(
        &self,
        address: PeerAddress,
        kind: SessionKind,
    ) -> Result<ValidatorRpcSession, ValidatorCommitteeClientError> {
        let peer_id = address.to_peer_id();
        let client = match kind {
            SessionKind::Pooled => self.pool.get_or_connect(&peer_id).await?,
            SessionKind::Dedicated => self.pool.connect_unpooled(&peer_id).await?,
        };
        Ok(ValidatorRpcSession {
            peer_address: address,
            client,
        })
    }

    /// Attempts to execute the provided callback with a session for a random committee member.
    /// If the callback succeeds, it returns the response.
    /// If the callback fails, it will try with another random member until all members have been attempted.
    /// If all members fail, it returns an error with the last error encountered.
    pub async fn try_with_random_members<F, T, E, TFut>(
        &mut self,
        callback: F,
    ) -> Result<T, ValidatorCommitteeClientError>
    where
        F: Fn(ValidatorRpcSession) -> TFut,
        TFut: Future<Output = Result<T, E>>,
        T: 'static,
        E: Display + 'static,
    {
        let epoch = self.epoch_manager.get_current_epoch();
        let mut last_error = None;
        let mut attempted = HashSet::new();
        loop {
            let Some(vn) = self
                .epoch_manager
                .get_random_committee_member(epoch, Some(self.shard_group), attempted.clone())
                .await
                .optional()?
            else {
                warn!(
                    target: LOG_TARGET,
                    "All {} committee members have been attempted and failed.", attempted.len()
                );
                // No more committee members to try
                break;
            };

            let session = match self.session_for_peer(vn.address, SessionKind::Pooled).await {
                Ok(session) => session,
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Failed to create session for validator '{}': {}", vn, err
                    );
                    last_error = Some(err.to_string());
                    attempted.insert(vn.address);
                    self.failed_peers.at(epoch).insert(vn.address);
                    continue; // Skip this member and try the next one
                },
            };

            match callback(session).await {
                Ok(ret) => {
                    return Ok(ret);
                },
                Err(err) => {
                    warn!(
                        target: LOG_TARGET,
                        "Request failed for validator '{}': {}", vn, err
                    );
                    last_error = Some(err.to_string());
                },
            }

            attempted.insert(vn.address);
        }

        Err(ValidatorCommitteeClientError::AllValidatorsFailed {
            committee_size: attempted.len(),
            last_error,
        })
    }
}

#[derive(Debug, Clone, Copy)]
enum SessionKind {
    Pooled,
    Dedicated,
}

/// Committee members that failed to connect, excluded from selection for the rest of the epoch they
/// failed in. Scoping the exclusion to an epoch lets a validator that was briefly unreachable (e.g.
/// restarting) back into selection once the epoch moves on.
#[derive(Debug, Clone, Default)]
struct FailedPeers {
    epoch: Epoch,
    peers: HashSet<PeerAddress>,
}

impl FailedPeers {
    /// The peers that have failed during `epoch`, forgetting any that failed in an earlier epoch.
    fn at(&mut self, epoch: Epoch) -> &mut HashSet<PeerAddress> {
        if self.epoch != epoch {
            self.epoch = epoch;
            self.clear();
        }
        &mut self.peers
    }

    fn clear(&mut self) {
        self.peers.clear();
        // Clamp max mem usage to 7300 bytes (Multihash size x 100) - this is likely to always be a no-op
        self.peers.shrink_to(100);
    }
}

#[derive(Clone)]
pub struct ValidatorRpcSession {
    peer_address: PeerAddress,
    client: rpc_service::ValidatorNodeRpcClient,
}

impl ValidatorRpcSession {
    pub fn peer_address(&self) -> &PeerAddress {
        &self.peer_address
    }

    pub fn client(&self) -> &rpc_service::ValidatorNodeRpcClient {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut rpc_service::ValidatorNodeRpcClient {
        &mut self.client
    }
}

impl Deref for ValidatorRpcSession {
    type Target = rpc_service::ValidatorNodeRpcClient;

    fn deref(&self) -> &Self::Target {
        self.client()
    }
}

impl DerefMut for ValidatorRpcSession {
    fn deref_mut(&mut self) -> &mut rpc_service::ValidatorNodeRpcClient {
        self.client_mut()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ValidatorCommitteeClientError {
    #[error("Epoch manager error: {0}")]
    EpochManagerError(#[from] EpochManagerError),
    #[error("Validator node RPC client error: {0}")]
    ValidatorNodeRpcClientError(#[from] ValidatorNodeRpcClientError),
    #[error("Rpc call failed for all ({committee_size}) validators: {}", .last_error.as_deref().unwrap_or("unknown"))]
    AllValidatorsFailed {
        committee_size: usize,
        last_error: Option<String>,
    },
}

#[cfg(test)]
mod tests {
    use tari_crypto::{
        keys::PublicKey,
        ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    };

    use super::*;

    fn peer(n: u64) -> PeerAddress {
        RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(n)).into()
    }

    #[test]
    fn failed_peers_are_excluded_for_the_rest_of_the_epoch() {
        let mut failed = FailedPeers::default();
        failed.at(Epoch(5)).insert(peer(1));
        failed.at(Epoch(5)).insert(peer(2));

        assert_eq!(*failed.at(Epoch(5)), HashSet::from([peer(1), peer(2)]));
    }

    #[test]
    fn failed_peers_are_forgotten_when_the_epoch_moves() {
        let mut failed = FailedPeers::default();
        failed.at(Epoch(5)).insert(peer(1));

        assert!(failed.at(Epoch(6)).is_empty());
        failed.at(Epoch(6)).insert(peer(2));
        assert_eq!(*failed.at(Epoch(6)), HashSet::from([peer(2)]));
    }
}
