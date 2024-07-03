//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use log::*;
use tari_dan_common_types::{shard::Shard, Epoch, SubstateAddress};
use tari_dan_p2p::proto::rpc::{
    sync_blocks_response::SyncData,
    QuorumCertificates,
    SyncBlocksResponse,
    SyncStateResponse,
    Transactions,
};
use tari_dan_storage::{
    consensus_models::{StateTransition, StateTransitionId},
    StateStore,
    StorageError,
};
use tari_rpc_framework::RpcStatus;
use tokio::sync::mpsc;

const LOG_TARGET: &str = "tari::dan::rpc::sync_task";

const BATCH_SIZE: usize = 100;

type UpdateBuffer = Vec<StateTransition>;

pub struct StateSyncTask<TStateStore: StateStore> {
    store: TStateStore,
    sender: mpsc::Sender<Result<SyncStateResponse, RpcStatus>>,
    start_state_transition_id: StateTransitionId,
    current_shard: Shard,
    current_epoch: Epoch,
}

impl<TStateStore: StateStore> StateSyncTask<TStateStore> {
    pub fn new(
        store: TStateStore,
        sender: mpsc::Sender<Result<SyncStateResponse, RpcStatus>>,
        start_state_transition_id: StateTransitionId,
        current_shard: Shard,
        current_epoch: Epoch,
    ) -> Self {
        Self {
            store,
            sender,
            start_state_transition_id,
            current_shard,
            current_epoch,
        }
    }

    pub async fn run(mut self) -> Result<(), ()> {
        let mut buffer = Vec::with_capacity(BATCH_SIZE);
        let mut current_state_transition_id = self.start_state_transition_id;
        let mut counter = 0;
        loop {
            match self.fetch_next_batch(&mut buffer, current_state_transition_id) {
                Ok(Some(last_state_transition_id)) => {
                    info!(target: LOG_TARGET, "🌍Fetched {} state transitions up to transition {}", buffer.len(), last_state_transition_id);
                    current_state_transition_id = last_state_transition_id;
                },
                Ok(None) => {
                    // TODO: differentiate between not found and end of stream
                    // self.send(Err(RpcStatus::not_found(format!(
                    //     "State transition not found with id={current_state_transition_id}"
                    // ))))
                    // .await?;

                    // Finished
                    return Ok(());
                },
                Err(err) => {
                    self.send(Err(RpcStatus::log_internal_error(LOG_TARGET)(err))).await?;
                    return Err(());
                },
            }

            let num_items = buffer.len();
            debug!(
                target: LOG_TARGET,
                "Sending {num_items} state updates to peer. Current transition id: {current_state_transition_id}",
            );

            counter += buffer.len();
            self.send_state_transitions(buffer.drain(..)).await?;

            // If we didn't fill up the buffer, so we're done
            if num_items < buffer.capacity() {
                debug!( target: LOG_TARGET, "Sync to last commit complete. Streamed {} item(s)", counter);
                break;
            }
        }

        Ok(())
    }

    fn fetch_next_batch(
        &self,
        buffer: &mut UpdateBuffer,
        current_state_transition_id: StateTransitionId,
    ) -> Result<Option<StateTransitionId>, StorageError> {
        self.store.with_read_tx(|tx| {
            let state_transitions = StateTransition::get_n_after(tx, BATCH_SIZE, current_state_transition_id)?;

            let Some(last) = state_transitions.last() else {
                return Ok(None);
            };

            let last_state_transition_id = last.id;
            buffer.extend(state_transitions);
            Ok::<_, StorageError>(Some(last_state_transition_id))
        })
    }

    async fn send(&mut self, result: Result<SyncStateResponse, RpcStatus>) -> Result<(), ()> {
        if self.sender.send(result).await.is_err() {
            debug!(
                target: LOG_TARGET,
                "Peer stream closed by client before completing. Aborting"
            );
            return Err(());
        }
        Ok(())
    }

    async fn send_state_transitions<I: IntoIterator<Item = StateTransition>>(
        &mut self,
        state_transitions: I,
    ) -> Result<(), ()> {
        self.send(Ok(SyncStateResponse {
            transitions: state_transitions.into_iter().map(Into::into).collect(),
            state_hash: vec![],
        }))
        .await?;

        Ok(())
    }
}
