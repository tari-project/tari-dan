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
use tokio::sync::mpsc;

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

pub struct ConsensusInboundMessaging<TMsgLogger, TRegistry = EpochManagerHandle<PeerAddress>> {
    local_address: PeerAddress,
    /// Direct messages, still encoded. Each is decoded only once taken off the queue, so the queue's byte budget
    /// bounds what it holds and decoding runs one message at a time.
    rx_inbound_msg: mpsc::Receiver<InboundMessage>,
    rx_gossip: mpsc::Receiver<(PeerId, HotstuffMessage)>,
    rx_loopback: mpsc::UnboundedReceiver<HotstuffMessage>,
    msg_logger: TMsgLogger,
    validators: TRegistry,
    /// A direct message taken off the queue whose sender is still being checked. It is held here rather than
    /// in the `next_message` future, so a caller that drops that future part way does not lose it.
    awaiting_sender_check: Option<(PeerId, proto::consensus::HotStuffMessage)>,
}

impl<TMsgLogger: MessageLogger, TRegistry: ValidatorRegistry> ConsensusInboundMessaging<TMsgLogger, TRegistry> {
    pub fn new(
        local_address: PeerAddress,
        rx_inbound_msg: mpsc::Receiver<InboundMessage>,
        rx_gossip: mpsc::Receiver<(PeerId, HotstuffMessage)>,
        rx_loopback: mpsc::UnboundedReceiver<HotstuffMessage>,
        msg_logger: TMsgLogger,
        validators: TRegistry,
    ) -> Self {
        Self {
            local_address,
            rx_inbound_msg,
            rx_gossip,
            rx_loopback,
            msg_logger,
            validators,
            awaiting_sender_check: None,
        }
    }

    fn handle_message(
        &self,
        from: PeerId,
        msg: proto::consensus::HotStuffMessage,
    ) -> Option<Result<(PeerAddress, HotstuffMessage), InboundMessagingError>> {
        match HotstuffMessage::try_from(msg) {
            Ok(msg) => {
                self.msg_logger
                    .log_inbound_message(&from.to_string(), msg.as_type_str(), "", &msg);
                Some(Ok((from.into(), msg)))
            },
            Err(err) => Some(Err(InboundMessagingError::InvalidMessage {
                reason: format!("from peer {from}: {err}"),
            })),
        }
    }
}

impl<TMsgLogger, TRegistry> tari_consensus::traits::InboundMessaging
    for ConsensusInboundMessaging<TMsgLogger, TRegistry>
where
    TMsgLogger: MessageLogger + Send,
    TRegistry: ValidatorRegistry + Send + Sync,
{
    type Addr = PeerAddress;

    async fn next_message(&mut self) -> Option<Result<(Self::Addr, HotstuffMessage), InboundMessagingError>> {
        if self.awaiting_sender_check.is_none() {
            tokio::select! {
                // BIASED: messaging priority is loopback, then other
                biased;
                maybe_msg = self.rx_loopback.recv() => return maybe_msg.map(|msg| {
                    self.msg_logger.log_inbound_message(
                       &self.local_address.to_string(),
                       msg.as_type_str(),
                       "",
                       &msg,
                    );
                    Ok((self.local_address, msg))
                }),
                maybe_msg = self.rx_inbound_msg.recv() => {
                    let InboundMessage { peer_id, message, .. } = maybe_msg?;
                    let message = match proto::consensus::HotStuffMessage::decode(message) {
                        Ok(message) => message,
                        Err(err) => {
                            return Some(Err(InboundMessagingError::InvalidMessage {
                                reason: format!("from peer {peer_id}: undecodable message: {err}"),
                            }));
                        },
                    };
                    if validator_only_epoch(&message).is_none() {
                        return self.handle_message(peer_id, message);
                    }
                    self.awaiting_sender_check = Some((peer_id, message));
                },
                maybe_msg = self.rx_gossip.recv() => {
                    let (from, msg) = maybe_msg?;
                    self.msg_logger
                        .log_inbound_message(&from.to_string(), msg.as_type_str(), "", &msg);
                    return Some(Ok((from.into(), msg)));
                },
            }
        }

        match resolve_sender_check(&mut self.awaiting_sender_check, &self.validators).await? {
            Ok((from, msg)) => self.handle_message(from, msg),
            Err(err) => Some(Err(err)),
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

/// Checks the sender of the message held in `pending`, releasing it only once the check completes. A caller that
/// drops this future part way leaves the message in `pending` for the next call to check again.
async fn resolve_sender_check<TRegistry: ValidatorRegistry>(
    pending: &mut Option<(PeerId, proto::consensus::HotStuffMessage)>,
    validators: &TRegistry,
) -> Option<Result<(PeerId, proto::consensus::HotStuffMessage), InboundMessagingError>> {
    let (from, msg) = pending.as_ref()?;
    let rejection = match validator_only_epoch(msg) {
        Some(epoch) if !validators.is_registered(epoch, &(*from).into()).await => Some(epoch),
        _ => None,
    };
    let (from, msg) = pending.take()?;
    match rejection {
        Some(epoch) => Some(Err(InboundMessagingError::InvalidMessage {
            reason: format!("peer {from} sent transactions for {epoch} but is not a validator registered for it"),
        })),
        None => Some(Ok((from, msg))),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::{HashMap, HashSet},
        sync::atomic::{AtomicBool, Ordering},
        time::Duration,
    };

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

    /// Never answers its first lookup, then answers every later one as registered.
    struct StallsOnce(AtomicBool);

    impl ValidatorRegistry for StallsOnce {
        async fn is_registered(&self, _epoch: Epoch, _address: &PeerAddress) -> bool {
            if !self.0.swap(true, Ordering::SeqCst) {
                std::future::pending::<()>().await;
            }
            true
        }
    }

    async fn check_sender<TRegistry: ValidatorRegistry>(
        validators: &TRegistry,
        from: PeerId,
        msg: HotStuffMessage,
    ) -> Result<(PeerId, HotStuffMessage), InboundMessagingError> {
        resolve_sender_check(&mut Some((from, msg)), validators)
            .await
            .expect("a message is pending")
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

        let err = check_sender(&registry, PeerId::random(), transactions_response())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a validator"), "{err}");
    }

    #[tokio::test]
    async fn a_transactions_response_from_a_registered_validator_is_admitted() {
        let validator = PeerId::random();
        let registry = Registered(HashSet::from([validator.into()]));

        check_sender(&registry, validator, transactions_response())
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn other_messages_are_admitted_from_any_peer() {
        let registry = Registered(HashSet::new());
        let request = HotStuffMessage {
            message: Some(Message::RequestMissingTransactions(Default::default())),
        };

        check_sender(&registry, PeerId::random(), request).await.unwrap();
    }

    #[tokio::test]
    async fn a_check_dropped_part_way_keeps_the_message_for_the_next_call() {
        let registry = StallsOnce(AtomicBool::new(false));
        let from = PeerId::random();
        let mut pending = Some((from, transactions_response()));

        let dropped =
            tokio::time::timeout(Duration::from_millis(10), resolve_sender_check(&mut pending, &registry)).await;
        assert!(dropped.is_err(), "the first lookup never answers");
        assert!(pending.is_some(), "the message survives the dropped check");

        let (resolved_from, _) = resolve_sender_check(&mut pending, &registry)
            .await
            .expect("the message is still pending")
            .unwrap();
        assert_eq!(resolved_from, from);
        assert!(pending.is_none());
    }

    /// The gossip and loopback senders, held so those channels stay open.
    type OpenChannels = (
        mpsc::Sender<(PeerId, HotstuffMessage)>,
        mpsc::UnboundedSender<HotstuffMessage>,
    );

    /// Direct messaging fed from a queue of the given byte budget.
    fn direct_messaging(
        max_queued_bytes: usize,
    ) -> (
        ConsensusInboundMessaging<NopLogger, Registered>,
        MessagingMode,
        MessageQueueSender,
        OpenChannels,
    ) {
        let (tx_messages, rx_messages) = message_queue(8, max_queued_bytes);
        let (tx_gossip, rx_gossip) = mpsc::channel(1);
        let (tx_loopback, rx_loopback) = mpsc::unbounded_channel();
        let messaging = ConsensusInboundMessaging::new(
            PeerId::random().into(),
            rx_messages,
            rx_gossip,
            rx_loopback,
            NopLogger,
            Registered(HashSet::new()),
        );
        let mode = MessagingMode::Enabled {
            tx_messages: tx_messages.clone(),
            tx_gossip_messages_by_topic: HashMap::new(),
        };
        (messaging, mode, tx_messages, (tx_gossip, tx_loopback))
    }

    #[tokio::test]
    async fn a_direct_message_is_decoded_once_taken_off_the_queue() {
        let (mut messaging, mode, queue, _open) = direct_messaging(1024);
        let from = PeerId::random();
        let request = HotStuffMessage {
            message: Some(Message::SyncRequest(SyncRequest {
                epoch: 3,
                block_height: 7,
            })),
        };
        let encoded = Bytes::from(prost::Message::encode_to_vec(&request));
        let len = encoded.len();

        mode.send_message(from, encoded).unwrap();
        assert_eq!(queue.queued_bytes(), len, "the queue charges the encoded message");

        let (sender, msg) = messaging.next_message().await.unwrap().unwrap();
        assert_eq!(sender, from);
        assert!(
            matches!(msg, HotstuffMessage::CatchUpSyncRequest(ref req) if req.epoch == Epoch(3)),
            "{msg:?}"
        );
        assert_eq!(queue.queued_bytes(), 0);
    }

    #[tokio::test]
    async fn an_undecodable_direct_message_is_refused_by_the_consumer() {
        let (mut messaging, mode, queue, _open) = direct_messaging(1024);

        mode.send_message(PeerId::random(), Bytes::from_static(&[0xff; 8]))
            .unwrap();

        let err = messaging.next_message().await.unwrap().unwrap_err();
        assert!(err.to_string().contains("undecodable"), "{err}");
        assert_eq!(queue.queued_bytes(), 0, "the refused message released its budget");
    }
}
