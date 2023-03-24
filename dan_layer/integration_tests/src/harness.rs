//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, str::FromStr, sync::Arc, time::Duration};

use tari_common_types::types::{PrivateKey, PublicKey};
use tari_comms::{multiaddr::Multiaddr, peer_manager::PeerFeatures, NodeIdentity};
use tari_dan_common_types::{ObjectPledge, ShardId};
use tari_dan_core::{
    consensus_constants::ConsensusConstants,
    models::{vote_message::VoteMessage, HotStuffMessage, Payload, TariDanPayload},
    services::{
        epoch_manager::EpochManager,
        leader_strategy::LeaderStrategy,
        NodeIdentitySigningService,
        PayloadProcessor,
        PayloadProcessorError,
        SigningService,
    },
    workers::{
        hotstuff_waiter::{HotStuffWaiter, RecoveryMessage},
        pacemaker_worker::Pacemaker,
    },
};
use tari_dan_engine::runtime::ConsensusContext;
use tari_engine_types::{
    commit_result::{ExecuteResult, FinalizeResult, RejectReason, TransactionResult},
    fees::FeeCostBreakdown,
    substate::SubstateDiff,
};
use tari_shutdown::Shutdown;
use tari_template_lib::{models::Amount, Hash};
use tokio::{
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
    },
    task::JoinHandle,
    time::timeout,
};

use crate::TempShardStoreFactory;

pub struct PayloadProcessorListener {
    pub receiver: broadcast::Receiver<(TariDanPayload, HashMap<ShardId, ObjectPledge>)>,
    sender: broadcast::Sender<(TariDanPayload, HashMap<ShardId, ObjectPledge>)>,
}

impl PayloadProcessorListener {
    pub fn new() -> Self {
        let (sender, receiver) = broadcast::channel(100);
        Self { receiver, sender }
    }
}

impl PayloadProcessor<TariDanPayload> for PayloadProcessorListener {
    fn process_payload(
        &self,
        payload: TariDanPayload,
        pledges: HashMap<ShardId, ObjectPledge>,
        _consensus: ConsensusContext,
    ) -> Result<ExecuteResult, PayloadProcessorError> {
        self.sender.send((payload, pledges)).unwrap();
        Ok(ExecuteResult {
            finalize: FinalizeResult::new(
                Hash::default(),
                vec![],
                TransactionResult::Accept(SubstateDiff::new()),
                FeeCostBreakdown {
                    total_fees_charged: Amount::zero(),
                    breakdown: vec![],
                },
            ),
            transaction_failure: None,
            fee_receipt: None,
        })
    }
}

impl Default for PayloadProcessorListener {
    fn default() -> Self {
        Self::new()
    }
}

pub struct NullPayloadProcessor {}

impl PayloadProcessor<TariDanPayload> for NullPayloadProcessor {
    fn process_payload(
        &self,
        payload: TariDanPayload,
        _pledges: HashMap<ShardId, ObjectPledge>,
        _consensus: ConsensusContext,
    ) -> Result<ExecuteResult, PayloadProcessorError> {
        Ok(ExecuteResult {
            finalize: FinalizeResult::reject(
                payload.to_id().into_array().into(),
                RejectReason::ExecutionFailure("NullPayloadProcessor".to_string()),
            ),
            transaction_failure: None,
            fee_receipt: None,
        })
    }
}

pub trait Consensus<TariDanPayload> {
    fn execute_transaction(
        &mut self,
        payload: TariDanPayload,
        inputs: Vec<ObjectPledge>,
        outputs: Vec<ObjectPledge>,
    ) -> Result<(), String>;
}

pub struct HsTestHarness {
    // TODO: Having a mix of pub and private fields is an anti-pattern (citation needed), need to spend some time
    // cleaning up the tests
    identity: PublicKey,
    pub tx_new: Sender<(TariDanPayload, ShardId)>,
    pub tx_hs_messages: Sender<(PublicKey, HotStuffMessage<TariDanPayload, PublicKey>)>,
    pub tx_recovery_messages: Sender<(PublicKey, RecoveryMessage)>,
    pub rx_leader: Receiver<(PublicKey, HotStuffMessage<TariDanPayload, PublicKey>)>,
    shutdown: Shutdown,
    pub rx_broadcast: Receiver<(HotStuffMessage<TariDanPayload, PublicKey>, Vec<PublicKey>)>,
    pub rx_recovery: Receiver<(RecoveryMessage, PublicKey)>,
    pub rx_recovery_broadcast: Receiver<(RecoveryMessage, Vec<PublicKey>)>,
    pub rx_vote_message: Receiver<(VoteMessage, PublicKey)>,
    pub tx_votes: Sender<(PublicKey, VoteMessage)>,
    rx_execute: broadcast::Receiver<(TariDanPayload, HashMap<ShardId, ObjectPledge>)>,
    shard_store: TempShardStoreFactory,
    hs_waiter: Option<JoinHandle<anyhow::Result<()>>>,
    signing_service: NodeIdentitySigningService,
}

impl HsTestHarness {
    pub fn new<TEpochManager, TLeader>(
        private_key: PrivateKey,
        identity: PublicKey,
        epoch_manager: TEpochManager,
        leader: TLeader,
        network_latency: Duration,
    ) -> Self
    where
        TEpochManager: EpochManager<PublicKey> + Send + Sync + 'static,
        TLeader: LeaderStrategy<PublicKey> + Send + Sync + 'static,
    {
        let (tx_new, rx_new) = channel(1);
        let (tx_hs_messages, rx_hs_messages) = channel(1);
        let (tx_recovery_messages, rx_recovery_messages) = channel(1);
        let (tx_leader, rx_leader) = channel(1);
        let (tx_broadcast, rx_broadcast) = channel(1);
        let (tx_recovery, rx_recovery) = channel(1);
        let (tx_recovery_broadcast, rx_recovery_broadcast) = channel(1);
        let (tx_vote_message, rx_vote_message) = channel(1);
        let (tx_votes, rx_votes) = channel(1);
        let (tx_events, _) = broadcast::channel(100);
        let payload_processor = PayloadProcessorListener::new();
        let rx_execute = payload_processor.receiver.resubscribe();
        let shutdown = Shutdown::new();

        let consensus_constants = ConsensusConstants::devnet();
        let shard_store = TempShardStoreFactory::new();

        let public_address = Multiaddr::from_str("/ip4/127.0.0.1/tcp/48000").unwrap();
        let node_identity = NodeIdentity::new(private_key, vec![public_address], PeerFeatures::COMMUNICATION_NODE);

        let pacemaker = Pacemaker::spawn(shutdown.to_signal());
        let signing_service = NodeIdentitySigningService::new(Arc::new(node_identity));
        let hs_waiter = HotStuffWaiter::spawn(
            signing_service.clone(),
            identity.clone(),
            epoch_manager,
            leader,
            rx_new,
            rx_hs_messages,
            rx_recovery_messages,
            rx_votes,
            tx_leader,
            tx_broadcast,
            tx_recovery,
            tx_recovery_broadcast,
            tx_vote_message,
            tx_events,
            pacemaker,
            payload_processor,
            shard_store.clone(),
            shutdown.to_signal(),
            consensus_constants,
            network_latency,
        );

        Self {
            identity,
            tx_new,
            tx_hs_messages,
            tx_recovery_messages,
            rx_leader,
            rx_recovery,
            shutdown,
            rx_broadcast,
            rx_recovery_broadcast,
            rx_vote_message,
            tx_votes,
            rx_execute,
            shard_store,
            hs_waiter: Some(hs_waiter),
            signing_service,
        }
    }

    pub fn state_store(&self) -> &TempShardStoreFactory {
        &self.shard_store
    }

    pub fn identity(&self) -> PublicKey {
        self.identity.clone()
    }

    pub fn signing_service(&self) -> &impl SigningService {
        &self.signing_service
    }

    pub async fn assert_shuts_down_safely(&mut self) {
        self.shutdown.trigger();
        self.hs_waiter
            .take()
            .unwrap()
            .await
            .expect("did not end cleanly")
            .unwrap();
    }

    pub async fn recv_broadcast(&mut self) -> (HotStuffMessage<TariDanPayload, PublicKey>, Vec<PublicKey>) {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_broadcast.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    pub async fn recv_vote_message(&mut self) -> (VoteMessage, PublicKey) {
        if let Some(msg) = timeout(Duration::from_secs(10), self.rx_vote_message.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    pub async fn recv_execute(&mut self) -> (TariDanPayload, HashMap<ShardId, ObjectPledge>) {
        if let Ok(msg) = timeout(Duration::from_secs(10), self.rx_execute.recv())
            .await
            .expect("timed out")
        {
            msg
        } else {
            // Otherwise there are no senders, meaning the main loop has shut down,
            // so try shutdown to get the actual error
            self.assert_shuts_down_safely().await;
            panic!("Shut down safely, but still received none");
        }
    }

    pub async fn assert_no_execute(&mut self) {
        assert!(
            timeout(Duration::from_secs(1), self.rx_execute.recv()).await.is_err(),
            "received an execute when we weren't expecting it"
        )
    }
}
