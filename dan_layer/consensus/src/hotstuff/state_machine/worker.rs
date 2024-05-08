//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{future::Future, marker::PhantomData, time::Duration};

use log::*;
use tari_shutdown::ShutdownSignal;
use tokio::{sync::watch, time};

use crate::{
    hotstuff::{
        state_machine::{
            event::ConsensusStateEvent,
            idle::Idle,
            state::{ConsensusCurrentState, ConsensusState},
        },
        HotStuffError,
        HotstuffWorker,
    },
    traits::{ConsensusSpec, SyncManager},
};

const LOG_TARGET: &str = "tari::dan::consensus::sm::worker";

#[derive(Debug)]
pub struct ConsensusWorker<TSpec> {
    pub(super) shutdown_signal: ShutdownSignal,
    _spec: PhantomData<TSpec>,
}

#[derive(Debug, Clone)]
pub struct ConsensusWorkerContextConfig {
    pub is_listener_mode: bool,
}

#[derive(Debug)]
pub struct ConsensusWorkerContext<TSpec: ConsensusSpec> {
    pub epoch_manager: TSpec::EpochManager,
    pub hotstuff: HotstuffWorker<TSpec>,
    pub state_sync: TSpec::SyncManager,
    pub tx_current_state: watch::Sender<ConsensusCurrentState>,
    pub config : ConsensusWorkerContextConfig
}

impl<TSpec> ConsensusWorker<TSpec>
where
    TSpec: ConsensusSpec,
    HotStuffError: From<<TSpec::SyncManager as SyncManager>::Error>,
{
    pub fn new( shutdown_signal: ShutdownSignal) -> Self {
        Self {
            shutdown_signal,
            _spec: PhantomData,
        }
    }

    async fn next_event(
        &self,
        context: &mut ConsensusWorkerContext<TSpec>,
        state: &ConsensusState<TSpec>,
    ) -> ConsensusStateEvent {
        match state {
            ConsensusState::Idle(state) => self.result_or_shutdown(state.on_enter(context)).await,
            ConsensusState::CheckSync(state) => self.result_or_shutdown(state.on_enter(context)).await,
            ConsensusState::Syncing(state) => self.result_or_shutdown(state.on_enter(context)).await,
            ConsensusState::Sleeping => {
                time::sleep(Duration::from_secs(5)).await;
                ConsensusStateEvent::Resume
            },
            ConsensusState::Running(state) => state
                .on_enter(context)
                .await
                .unwrap_or_else(|err| ConsensusStateEvent::Failure { error: err }),
            ConsensusState::Shutdown => ConsensusStateEvent::Shutdown,
        }
    }

    fn transition(&mut self, state: ConsensusState<TSpec>, event: ConsensusStateEvent) -> ConsensusState<TSpec> {
        let state_str = state.to_string();
        let event_str = event.to_string();

        let next_state = match (state, event) {
            (ConsensusState::Idle(state), ConsensusStateEvent::RegisteredForEpoch { .. }) => {
                ConsensusState::CheckSync(state.into())
            },
            (ConsensusState::Idle(state), ConsensusStateEvent::ListenerMode) => {
                ConsensusState::CheckSync(state.into())
            },
            (ConsensusState::CheckSync(state), ConsensusStateEvent::NeedSync) => ConsensusState::Syncing(state.into()),
            (ConsensusState::CheckSync(state), ConsensusStateEvent::Ready) => ConsensusState::Running(state.into()),
            (ConsensusState::Syncing(state), ConsensusStateEvent::SyncComplete) => {
                ConsensusState::Running(state.into())
            },
            (ConsensusState::Sleeping, ConsensusStateEvent::Resume) => ConsensusState::Idle(Idle::new()),
            (ConsensusState::Running(state), ConsensusStateEvent::NeedSync) => ConsensusState::CheckSync(state.into()),
            (ConsensusState::Running(state), ConsensusStateEvent::NotRegisteredForEpoch { .. }) => {
                ConsensusState::Idle(state.into())
            },
            (_, ConsensusStateEvent::Failure { error }) => {
                error!(target: LOG_TARGET, "🚨 Failure: {}", error);
                ConsensusState::Sleeping
            },
            (_, ConsensusStateEvent::Shutdown) => ConsensusState::Shutdown,
            (state, event) => unreachable!("Invalid state transition from {} via {}", state, event),
        };

        info!(target: LOG_TARGET, "⚙️ TRANSITION: {state_str} --- {event_str} ---> {next_state}");
        next_state
    }

    pub fn spawn(
        mut self,
        context: ConsensusWorkerContext<TSpec>,
    ) -> tokio::task::JoinHandle<Result<(), anyhow::Error>> {
        tokio::spawn(async move {
            self.run(context).await;
            Ok(())
        })
    }

    pub async fn run(&mut self, mut context: ConsensusWorkerContext<TSpec>) {
        let mut state = ConsensusState::Idle(Idle::new());
        loop {
            let next_event = self.next_event(&mut context, &state).await;
            state = self.transition(state, next_event);
            let _ignore = context.tx_current_state.send((&state).into());
            if state.is_shutdown() {
                break;
            }
        }
    }

    async fn result_or_shutdown<Fut>(&self, fut: Fut) -> ConsensusStateEvent
    where Fut: Future<Output = Result<ConsensusStateEvent, HotStuffError>> {
        let mut shutdown_signal = self.shutdown_signal.clone();
        let result = tokio::select! {
            _ = shutdown_signal.wait() => Ok(ConsensusStateEvent::Shutdown),
            ret = fut => ret,
        };

        result.unwrap_or_else(|err| ConsensusStateEvent::Failure { error: err })
    }
}
