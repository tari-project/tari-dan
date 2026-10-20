//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io, sync::Arc};

use libp2p::{
    PeerId,
    futures::{AsyncRead, AsyncWrite, SinkExt, StreamExt},
};

use crate::{
    Config,
    Error,
    Event,
    SignedPeerRecord,
    behaviour::WantList,
    handler::FramedOutbound,
    proto,
    store::PeerStore,
};

pub async fn outbound_request_want_list_task<TPeerStore: PeerStore>(
    peer_id: PeerId,
    mut framed: FramedOutbound,
    store: TPeerStore,
    want_list: Arc<WantList>,
    local_peer_record: Option<Arc<SignedPeerRecord>>,
    config: Config,
) -> Event {
    tracing::debug!("Starting outbound protocol with peer {}", peer_id);
    outbound_request_want_list_task_inner(
        peer_id,
        &mut framed,
        store,
        want_list,
        local_peer_record.as_deref(),
        &config,
    )
    .await
    .unwrap_or_else(Event::Error)
}

async fn outbound_request_want_list_task_inner<TPeerStore, S>(
    peer_id: PeerId,
    framed: &mut FramedOutbound<S>,
    store: TPeerStore,
    want_list: Arc<WantList>,
    local_peer_record: Option<&SignedPeerRecord>,
    config: &Config,
) -> Result<Event, Error>
where
    TPeerStore: PeerStore,
    S: AsyncRead + AsyncWrite + Unpin,
{
    if let Some(local_peer_record) = local_peer_record {
        tracing::debug!("Sending updated local peer record to peer {}", peer_id);
        let msg = proto::SignedPeerRecord::from(local_peer_record);
        framed.send(msg.into()).await.map_err(|e| Error::CodecError(e.into()))?;
    }

    if want_list.is_empty() {
        tracing::debug!("[peer_id={peer_id}] Empty want list. Protocol complete");
        // Nothing further to do, let the peer know
        framed
            .send(proto::Message {
                payload: proto::mod_Message::OneOfpayload::None,
            })
            .await
            .map_err(|e| Error::CodecError(e.into()))?;
        return Ok(Event::InboundRequestCompleted {
            peer_id,
            peers_sent: 1,
            requested: 0,
        });
    }

    framed
        .send(
            proto::WantPeers {
                want_peer_ids: want_list.iter().map(|p| p.to_bytes()).collect(),
            }
            .into(),
        )
        .await
        .map_err(|e| Error::CodecError(e.into()))?;
    tracing::debug!("Sent want list (size={}) to peer {}", want_list.len(), peer_id);

    let mut new_peers = 0;
    let mut received_responses = 0;
    while let Some(msg) = framed.next().await {
        received_responses += 1;
        if received_responses > want_list.len() {
            return Err(Error::InvalidMessage {
                peer_id,
                details: format!("Peer {peer_id} sent us more peers than we requested"),
            });
        }

        match msg {
            Ok(msg) => {
                let Some(rec) = validate_want_peer_response(peer_id, msg, &want_list, config)? else {
                    continue;
                };

                new_peers += 1;

                store
                    .put_if_newer(rec)
                    .await
                    .map_err(|err| Error::StoreError(err.to_string()))?;
            },
            Err(e) => {
                let e = io::Error::from(e);
                if e.kind() == io::ErrorKind::UnexpectedEof {
                    return Ok(Event::OutboundStreamInterrupted { peer_id });
                } else {
                    return Err(Error::CodecError(e));
                }
            },
        }
    }

    tracing::debug!("Received {} new peers from {}", new_peers, peer_id);

    Ok(Event::PeerBatchReceived {
        from_peer: peer_id,
        new_peers,
    })
}

/// Validates a single want-list response, returning `None` if the record should be skipped.
fn validate_want_peer_response(
    peer_id: PeerId,
    msg: proto::WantPeerResponse,
    want_list: &WantList,
    config: &Config,
) -> Result<Option<SignedPeerRecord>, Error> {
    let Some(peer) = msg.peer else {
        return Err(Error::InvalidMessage {
            peer_id,
            details: "empty message".to_string(),
        });
    };

    let rec = match SignedPeerRecord::try_from(peer) {
        Ok(rec) => rec,
        Err(e) => {
            return Err(Error::InvalidMessage {
                peer_id,
                details: e.to_string(),
            });
        },
    };

    let subject_peer_id = rec.to_peer_id();
    if !want_list.contains(&subject_peer_id) {
        return Err(Error::InvalidMessage {
            peer_id,
            details: format!("Peer {peer_id} sent us a peer we didnt request"),
        });
    }

    if !rec.is_valid() {
        return Err(Error::InvalidSignedPeer {
            peer_id,
            details: format!("Peer {peer_id} returned an invalid signed peer record for peer {subject_peer_id}"),
        });
    }

    if !rec.is_updated_at_within(config.max_clock_drift) {
        tracing::debug!(
            "Skipping peer record for {subject_peer_id} from {peer_id}: updated_at {:?} is more than {:?} ahead of \
             local time",
            rec.updated_at,
            config.max_clock_drift,
        );
        return Ok(None);
    }

    Ok(Some(rec))
}

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
        time::Duration,
    };

    use asynchronous_codec::{BytesMut, Encoder};
    use libp2p::{
        futures::{executor::block_on, io::Cursor},
        identity,
    };

    use super::*;
    use crate::{LocalPeerRecord, MAX_MESSAGE_SIZE, PeerSignature, epoch_time::epoch_time_now, store::MemoryPeerStore};

    struct DuplexBuffer {
        read: Cursor<Vec<u8>>,
        written: Vec<u8>,
    }

    impl AsyncRead for DuplexBuffer {
        fn poll_read(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut [u8]) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.read).poll_read(cx, buf)
        }
    }

    impl AsyncWrite for DuplexBuffer {
        fn poll_write(mut self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
            Pin::new(&mut self.written).poll_write(cx, buf)
        }

        fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.written).poll_flush(cx)
        }

        fn poll_close(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Pin::new(&mut self.written).poll_close(cx)
        }
    }

    fn make_outbound_framed(responses: Vec<proto::WantPeerResponse>) -> FramedOutbound<DuplexBuffer> {
        let mut codec = quick_protobuf_codec::Codec::<proto::WantPeerResponse, proto::Message>::new(MAX_MESSAGE_SIZE);
        let mut buf = BytesMut::new();
        for resp in responses {
            codec.encode(resp, &mut buf).unwrap();
        }
        FramedOutbound::new(
            DuplexBuffer {
                read: Cursor::new(buf.to_vec()),
                written: Vec::new(),
            },
            quick_protobuf_codec::Codec::new(MAX_MESSAGE_SIZE),
        )
    }

    #[test]
    fn outbound_rejects_bad_signature_and_attributes_error_to_responder() {
        block_on(async {
            let responder_peer = PeerId::random();
            let subject_kp = Arc::new(identity::Keypair::generate_ed25519());
            let mut subject_rec = LocalPeerRecord::new(subject_kp);
            subject_rec.add_address("/ip4/127.0.0.1/tcp/9100".parse().unwrap());
            let mut bad_rec = SignedPeerRecord::from(subject_rec);
            let subject_peer = bad_rec.to_peer_id();
            bad_rec.signature.signature[0] ^= 0xff;

            let mut framed = make_outbound_framed(vec![proto::WantPeerResponse {
                peer: Some(bad_rec.into()),
            }]);
            let store = MemoryPeerStore::new();
            let want_list = Arc::new([subject_peer].into_iter().collect());

            let err = outbound_request_want_list_task_inner(
                responder_peer,
                &mut framed,
                store.clone(),
                want_list,
                None,
                &Config::default(),
            )
            .await
            .unwrap_err();

            match err {
                Error::InvalidSignedPeer { peer_id, details } => {
                    assert_eq!(peer_id, responder_peer);
                    assert!(details.contains(&subject_peer.to_string()), "details={details}");
                },
                other => panic!("expected InvalidSignedPeer, got {other:?}"),
            }
            assert!(store.get(&subject_peer).await.unwrap().is_none());
        });
    }

    #[test]
    fn outbound_skips_clock_skewed_record_without_aborting_batch() {
        block_on(async {
            let responder_peer = PeerId::random();

            // Record 1: timestamp skewed past max_clock_drift
            let skewed_kp = identity::Keypair::generate_ed25519();
            let skewed_addrs = vec!["/ip4/127.0.0.1/tcp/9101".parse().unwrap()];
            let far_future = epoch_time_now() + Duration::from_secs(10_000);
            let skewed_sig = PeerSignature::sign(
                &skewed_kp,
                &crate::peer_record::peer_signature_challenge(&skewed_addrs, &far_future),
            );
            let skewed_rec = SignedPeerRecord {
                addresses: skewed_addrs,
                updated_at: far_future,
                signature: skewed_sig,
            };
            let skewed_peer = skewed_rec.to_peer_id();

            // Record 2: valid timestamp
            let honest_kp = Arc::new(identity::Keypair::generate_ed25519());
            let mut honest_local = LocalPeerRecord::new(honest_kp);
            honest_local.add_address("/ip4/127.0.0.1/tcp/9102".parse().unwrap());
            let honest_rec = SignedPeerRecord::from(honest_local);
            let honest_peer = honest_rec.to_peer_id();

            let mut framed = make_outbound_framed(vec![
                proto::WantPeerResponse {
                    peer: Some(skewed_rec.into()),
                },
                proto::WantPeerResponse {
                    peer: Some(honest_rec.into()),
                },
            ]);
            let store = MemoryPeerStore::new();
            let want_list = Arc::new([skewed_peer, honest_peer].into_iter().collect());

            let event = outbound_request_want_list_task_inner(
                responder_peer,
                &mut framed,
                store.clone(),
                want_list,
                None,
                &Config::default(),
            )
            .await
            .unwrap();

            assert!(matches!(
                event,
                Event::PeerBatchReceived {
                    from_peer,
                    new_peers: 1
                } if from_peer == responder_peer
            ));
            assert!(store.get(&skewed_peer).await.unwrap().is_none());
            assert!(store.get(&honest_peer).await.unwrap().is_some());
        });
    }

    #[test]
    fn outbound_caps_skipped_clock_skewed_records_at_want_list_len() {
        block_on(async {
            let responder_peer = PeerId::random();

            let skewed_kp = identity::Keypair::generate_ed25519();
            let skewed_addrs = vec!["/ip4/127.0.0.1/tcp/9103".parse().unwrap()];
            let far_future = epoch_time_now() + Duration::from_secs(10_000);
            let skewed_sig = PeerSignature::sign(
                &skewed_kp,
                &crate::peer_record::peer_signature_challenge(&skewed_addrs, &far_future),
            );
            let skewed_rec = SignedPeerRecord {
                addresses: skewed_addrs,
                updated_at: far_future,
                signature: skewed_sig,
            };
            let skewed_peer = skewed_rec.to_peer_id();

            let want_list = Arc::new([skewed_peer].into_iter().collect::<WantList>());
            let responses = (0..=want_list.len())
                .map(|_| proto::WantPeerResponse {
                    peer: Some(skewed_rec.clone().into()),
                })
                .collect();
            let mut framed = make_outbound_framed(responses);
            let store = MemoryPeerStore::new();

            let err = outbound_request_want_list_task_inner(
                responder_peer,
                &mut framed,
                store.clone(),
                want_list,
                None,
                &Config::default(),
            )
            .await
            .unwrap_err();

            match err {
                Error::InvalidMessage { peer_id, details } => {
                    assert_eq!(peer_id, responder_peer);
                    assert!(
                        details.contains("sent us more peers than we requested"),
                        "details={details}"
                    );
                },
                other => panic!("expected InvalidMessage, got {other:?}"),
            }
            assert!(store.get(&skewed_peer).await.unwrap().is_none());
        });
    }
}
