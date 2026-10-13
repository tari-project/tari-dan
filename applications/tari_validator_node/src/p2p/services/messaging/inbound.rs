//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::future::Future;

use libp2p::PeerId;
use prost::Message;
use tari_consensus::{messages::HotstuffMessage, traits::InboundMessagingError};
use tari_epoch_manager::{EpochManagerReader, service::EpochManagerHandle};
use tari_networking::InboundMessage;
use tari_ootle_common_types::Epoch;
use tari_ootle_p2p::{PeerAddress, proto};
use tokio::{
    sync::mpsc,
    task::{self, JoinHandle},
};

use crate::p2p::logging::MessageLogger;

/// Answers whether a peer is a validator registered for an epoch.
pub trait ValidatorRegistry {
    fn is_registered(&self, epoch: Epoch, address: &PeerAddress) -> impl Future<Output = bool> + Send;
}

impl ValidatorRegistry for EpochManagerHandle<PeerAddress> {
    async fn is_registered(&self, epoch: Epoch, address: &PeerAddress) -> bool {
        self.get_committee_info_by_validator_address(epoch, address)
            .await
            .is_ok()
    }
}

/// A direct message decoded from the bytes it arrived as, or the reason it was refused.
pub type DirectMessageResult = Result<(PeerId, HotstuffMessage), InboundMessagingError>;

/// Spawns the task that decodes direct messages taken off the inbound queue, one at a time, ahead of consensus.
///
/// A decoded message can occupy many times the bytes it arrived as, and decoding a large one takes time, so it runs
/// on this task and the consensus worker only ever receives decoded messages. The task decodes a message only once
/// consensus has taken the previous one, so at most one decoded message waits for it.
pub fn spawn_direct_message_decoder<TRegistry>(
    rx_inbound: mpsc::Receiver<InboundMessage>,
    validators: TRegistry,
) -> (mpsc::Receiver<DirectMessageResult>, JoinHandle<anyhow::Result<()>>)
where
    TRegistry: ValidatorRegistry + Send + Sync + 'static,
{
    let (tx_decoded, rx_decoded) = mpsc::channel(1);
    let join_handle = task::spawn(decode_direct_messages(rx_inbound, tx_decoded, validators));
    (rx_decoded, join_handle)
}

async fn decode_direct_messages<TRegistry: ValidatorRegistry>(
    mut rx_inbound: mpsc::Receiver<InboundMessage>,
    tx_decoded: mpsc::Sender<DirectMessageResult>,
    validators: TRegistry,
) -> anyhow::Result<()> {
    loop {
        let Ok(slot) = tx_decoded.reserve().await else {
            return Ok(());
        };
        let inbound = tokio::select! {
            _ = tx_decoded.closed() => return Ok(()),
            maybe_msg = rx_inbound.recv() => match maybe_msg {
                Some(inbound) => inbound,
                None => return Ok(()),
            },
        };
        slot.send(decode_direct_message(inbound, &validators).await);
    }
}

async fn decode_direct_message<TRegistry: ValidatorRegistry>(
    inbound: InboundMessage,
    validators: &TRegistry,
) -> DirectMessageResult {
    let InboundMessage {
        peer_id: from, message, ..
    } = inbound;
    let invalid = |reason: String| InboundMessagingError::InvalidMessage {
        reason: format!("from peer {from}: {reason}"),
    };
    let msg = proto::consensus::HotStuffMessage::decode(message)
        .map_err(|err| invalid(format!("undecodable message: {err}")))?;
    check_sender(from, &msg, validators).await?;
    let msg = HotstuffMessage::try_from(msg).map_err(|err| invalid(err.to_string()))?;
    Ok((from, msg))
}

pub struct ConsensusInboundMessaging<TMsgLogger> {
    local_address: PeerAddress,
    rx_direct: mpsc::Receiver<DirectMessageResult>,
    rx_gossip: mpsc::Receiver<(PeerId, HotstuffMessage)>,
    rx_loopback: mpsc::UnboundedReceiver<HotstuffMessage>,
    msg_logger: TMsgLogger,
}

impl<TMsgLogger: MessageLogger> ConsensusInboundMessaging<TMsgLogger> {
    pub fn new(
        local_address: PeerAddress,
        rx_direct: mpsc::Receiver<DirectMessageResult>,
        rx_gossip: mpsc::Receiver<(PeerId, HotstuffMessage)>,
        rx_loopback: mpsc::UnboundedReceiver<HotstuffMessage>,
        msg_logger: TMsgLogger,
    ) -> Self {
        Self {
            local_address,
            rx_direct,
            rx_gossip,
            rx_loopback,
            msg_logger,
        }
    }

    fn log_inbound(&self, from: &PeerId, msg: &HotstuffMessage) {
        self.msg_logger
            .log_inbound_message(&from.to_string(), msg.as_type_str(), "", msg);
    }
}

impl<TMsgLogger> tari_consensus::traits::InboundMessaging for ConsensusInboundMessaging<TMsgLogger>
where TMsgLogger: MessageLogger + Send
{
    type Addr = PeerAddress;

    async fn next_message(&mut self) -> Option<Result<(Self::Addr, HotstuffMessage), InboundMessagingError>> {
        tokio::select! {
            // BIASED: messaging priority is loopback, then other
            biased;
            maybe_msg = self.rx_loopback.recv() => maybe_msg.map(|msg| {
                self.msg_logger.log_inbound_message(
                   &self.local_address.to_string(),
                   msg.as_type_str(),
                   "",
                   &msg,
                );
                Ok((self.local_address, msg))
            }),
            maybe_msg = self.rx_direct.recv() => {
                let (from, msg) = match maybe_msg? {
                    Ok(decoded) => decoded,
                    Err(err) => return Some(Err(err)),
                };
                self.log_inbound(&from, &msg);
                Some(Ok((from.into(), msg)))
            },
            maybe_msg = self.rx_gossip.recv() => {
                let (from, msg) = maybe_msg?;
                self.log_inbound(&from, &msg);
                Some(Ok((from.into(), msg)))
            },
        }
    }
}

/// The epoch whose registered validators alone may send `msg`, for a kind only a validator sends. A
/// transactions response is only ever requested from a validator, and its transactions are costly to decode, so
/// its sender is checked before they are.
fn validator_only_epoch(msg: &proto::consensus::HotStuffMessage) -> Option<Epoch> {
    match &msg.message {
        Some(proto::consensus::hot_stuff_message::Message::RequestedTransaction(response)) => {
            Some(Epoch(response.epoch))
        },
        _ => None,
    }
}

/// Refuses `msg` when only a validator registered for its epoch may send it and `from` is not one.
async fn check_sender<TRegistry: ValidatorRegistry>(
    from: PeerId,
    msg: &proto::consensus::HotStuffMessage,
    validators: &TRegistry,
) -> Result<(), InboundMessagingError> {
    match validator_only_epoch(msg) {
        Some(epoch) if !validators.is_registered(epoch, &from.into()).await => {
            Err(InboundMessagingError::InvalidMessage {
                reason: format!("peer {from} sent transactions for {epoch} but is not a validator registered for it"),
            })
        },
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use prost::bytes::Bytes;
    use tari_consensus::traits::InboundMessaging;
    use tari_networking::{MessageQueueSender, MessagingMode, message_queue};
    use tari_ootle_p2p::proto::{
        consensus::{HotStuffMessage, MissingTransactionsResponse, SyncRequest, hot_stuff_message::Message},
        transaction::Transaction,
    };

    use super::*;
    use crate::p2p::logging::NopLogger;

    struct Registered(HashSet<PeerAddress>);

    impl ValidatorRegistry for Registered {
        async fn is_registered(&self, _epoch: Epoch, address: &PeerAddress) -> bool {
            self.0.contains(address)
        }
    }

    fn transactions_response() -> HotStuffMessage {
        HotStuffMessage {
            message: Some(Message::RequestedTransaction(MissingTransactionsResponse {
                request_id: 1,
                epoch: 1,
                block_id: vec![1; 32],
                transactions: vec![Transaction {
                    bor_encoded: vec![0xff; 16],
                }],
            })),
        }
    }

    #[tokio::test]
    async fn a_transactions_response_from_an_unregistered_peer_is_refused() {
        let validator = PeerId::random();
        let registry = Registered(HashSet::from([validator.into()]));

        let err = check_sender(PeerId::random(), &transactions_response(), &registry)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a validator"), "{err}");
    }

    #[tokio::test]
    async fn a_transactions_response_from_a_registered_validator_is_admitted() {
        let validator = PeerId::random();
        let registry = Registered(HashSet::from([validator.into()]));

        check_sender(validator, &transactions_response(), &registry)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn other_messages_are_admitted_from_any_peer() {
        let registry = Registered(HashSet::new());
        let request = HotStuffMessage {
            message: Some(Message::RequestMissingTransactions(Default::default())),
        };

        check_sender(PeerId::random(), &request, &registry).await.unwrap();
    }

    /// The gossip and loopback senders, held so those channels stay open.
    type OpenChannels = (
        mpsc::Sender<(PeerId, HotstuffMessage)>,
        mpsc::UnboundedSender<HotstuffMessage>,
    );

    /// Direct messaging fed through the decoder from a queue of the given byte budget.
    fn direct_messaging(
        max_queued_bytes: usize,
    ) -> (
        ConsensusInboundMessaging<NopLogger>,
        MessagingMode,
        MessageQueueSender,
        OpenChannels,
    ) {
        let (tx_messages, rx_messages) = message_queue(8, max_queued_bytes);
        let (rx_direct, _decoder) = spawn_direct_message_decoder(rx_messages, Registered(HashSet::new()));
        let (tx_gossip, rx_gossip) = mpsc::channel(1);
        let (tx_loopback, rx_loopback) = mpsc::unbounded_channel();
        let messaging =
            ConsensusInboundMessaging::new(PeerId::random().into(), rx_direct, rx_gossip, rx_loopback, NopLogger);
        let mode = MessagingMode::Enabled {
            tx_messages: tx_messages.clone(),
            tx_gossip_messages_by_topic: HashMap::new(),
        };
        (messaging, mode, tx_messages, (tx_gossip, tx_loopback))
    }

    fn encoded_sync_request(epoch: u64) -> Bytes {
        let request = HotStuffMessage {
            message: Some(Message::SyncRequest(SyncRequest { epoch, block_height: 7 })),
        };
        Bytes::from(prost::Message::encode_to_vec(&request))
    }

    /// Yields to the runtime until the decoder has run as far as it can.
    async fn let_decoder_settle() {
        for _ in 0..100 {
            tokio::task::yield_now().await;
        }
    }

    #[tokio::test]
    async fn a_direct_message_is_decoded_once_taken_off_the_queue() {
        let (mut messaging, mode, queue, _open) = direct_messaging(1024);
        let from = PeerId::random();

        mode.send_message(from, encoded_sync_request(3)).unwrap();

        let (sender, msg) = messaging.next_message().await.unwrap().unwrap();
        assert_eq!(sender, from);
        assert!(
            matches!(msg, HotstuffMessage::CatchUpSyncRequest(ref req) if req.epoch == Epoch(3)),
            "{msg:?}"
        );
        assert_eq!(queue.queued_bytes(), 0);
    }

    #[tokio::test]
    async fn the_decoder_runs_at_most_one_message_ahead_of_consensus() {
        let (mut messaging, mode, queue, _open) = direct_messaging(1024);
        let len = encoded_sync_request(1).len();
        for epoch in 1..=3 {
            mode.send_message(PeerId::random(), encoded_sync_request(epoch))
                .unwrap();
        }

        let_decoder_settle().await;
        assert_eq!(
            queue.queued_bytes(),
            2 * len,
            "one message is decoded ahead and the rest stay encoded in the queue"
        );

        let (_, first) = messaging.next_message().await.unwrap().unwrap();
        assert!(matches!(first, HotstuffMessage::CatchUpSyncRequest(ref req) if req.epoch == Epoch(1)));
        let_decoder_settle().await;
        assert_eq!(queue.queued_bytes(), len);
    }

    #[tokio::test]
    async fn an_undecodable_direct_message_is_refused() {
        let (mut messaging, mode, queue, _open) = direct_messaging(1024);

        mode.send_message(PeerId::random(), Bytes::from_static(&[0xff; 8]))
            .unwrap();

        let err = messaging.next_message().await.unwrap().unwrap_err();
        assert!(err.to_string().contains("undecodable"), "{err}");
        assert_eq!(queue.queued_bytes(), 0, "the refused message released its budget");
    }
}
