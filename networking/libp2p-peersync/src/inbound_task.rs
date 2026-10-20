//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashSet, io};

use libp2p::{
    PeerId,
    futures::{AsyncRead, AsyncWrite, SinkExt, StreamExt},
};

use crate::{Config, Error, Event, SignedPeerRecord, handler::FramedInbound, proto, store::PeerStore};

pub async fn inbound_sync_task<TPeerStore: PeerStore>(
    peer_id: PeerId,
    framed: FramedInbound,
    store: TPeerStore,
    config: Config,
) -> Event {
    tracing::debug!("Starting inbound protocol sync with peer {}", peer_id);
    inbound_sync_task_inner(peer_id, framed, store, config)
        .await
        .unwrap_or_else(Event::Error)
}

async fn inbound_sync_task_inner<TPeerStore, S>(
    peer_id: PeerId,
    mut framed: FramedInbound<S>,
    store: TPeerStore,
    config: Config,
) -> Result<Event, Error>
where
    TPeerStore: PeerStore,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut received_remote_peer_record = false;
    loop {
        let msg = framed.next().await.ok_or(Error::InboundStreamEnded)??;

        match msg.payload {
            proto::mod_Message::OneOfpayload::LocalRecord(msg) => {
                // Only permitted once per session
                if received_remote_peer_record {
                    return Err(Error::InvalidMessage {
                        peer_id,
                        details: format!("peer {peer_id} sent more than one local peer record"),
                    });
                }

                let msg = SignedPeerRecord::try_from(msg)?;
                tracing::debug!(
                    "Received local peer record from peer {peer_id} containing {} address(es)",
                    msg.addresses.len()
                );
                if msg.to_peer_id() != peer_id {
                    return Err(Error::InvalidSignedPeer {
                        peer_id,
                        details: format!(
                            "peer {peer_id} sent local record for different peer {}",
                            msg.to_peer_id()
                        ),
                    });
                }
                if !msg.is_valid() {
                    return Err(Error::InvalidSignedPeer {
                        peer_id,
                        details: format!("peer {peer_id} sent an invalid local peer record"),
                    });
                }
                if !msg.is_updated_at_within(config.max_clock_drift) {
                    return Err(Error::InvalidSignedPeer {
                        peer_id,
                        details: format!(
                            "peer {peer_id} sent a local peer record whose updated_at {:?} is more than {:?} ahead of \
                             local time",
                            msg.updated_at, config.max_clock_drift
                        ),
                    });
                }
                store.put(msg).await.map_err(|e| Error::StoreError(e.to_string()))?;
                received_remote_peer_record = true;
            },
            proto::mod_Message::OneOfpayload::WantPeers(msg) => {
                tracing::debug!(
                    "Want peer request (size={}) from peer {peer_id}",
                    msg.want_peer_ids.len()
                );
                return handle_want_peers(peer_id, &mut framed, &store, &config, msg).await;
            },
            proto::mod_Message::OneOfpayload::None => {
                return Ok(Event::InboundRequestCompleted {
                    peer_id,
                    peers_sent: 0,
                    requested: 0,
                });
            },
        }
    }
}

async fn handle_want_peers<TPeerStore, S>(
    peer_id: PeerId,
    framed: &mut FramedInbound<S>,
    store: &TPeerStore,
    config: &Config,
    msg: proto::WantPeers,
) -> Result<Event, Error>
where
    TPeerStore: PeerStore,
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut store_stream = store.stream();

    let orig_want_list_len = msg.want_peer_ids.len();
    if orig_want_list_len > config.max_want_list_len {
        tracing::warn!(
            "Peer {} requested {} peers, but the maximum is {}",
            peer_id,
            orig_want_list_len,
            config.max_want_list_len
        );
        return Err(Error::WantListTooLarge {
            want_list_len: orig_want_list_len,
            max_len: config.max_want_list_len,
        });
    }

    let mut remaining_want_list = msg
        .want_peer_ids
        .into_iter()
        .map(|p| PeerId::from_bytes(&p))
        .collect::<Result<HashSet<_>, _>>()
        .map_err(|e| Error::InvalidMessage {
            peer_id,
            details: format!("invalid peer id in requested want_list: {e}"),
        })?;

    let event = loop {
        if remaining_want_list.is_empty() {
            break Event::InboundRequestCompleted {
                peer_id,
                peers_sent: orig_want_list_len - remaining_want_list.len(),
                requested: orig_want_list_len,
            };
        }

        let Some(result) = store_stream.next().await else {
            break Event::InboundRequestCompleted {
                peer_id,
                peers_sent: orig_want_list_len - remaining_want_list.len(),
                requested: orig_want_list_len,
            };
        };

        let synced_peer: SignedPeerRecord = result.map_err(|e| Error::StoreError(e.to_string()))?;
        let synced_peer_id = synced_peer.to_peer_id();

        if !remaining_want_list.remove(&synced_peer_id) {
            continue;
        }

        if let Err(e) = framed
            .send(proto::WantPeerResponse {
                peer: Some(synced_peer.into()),
            })
            .await
        {
            let e = io::Error::from(e);
            if e.kind() == io::ErrorKind::UnexpectedEof {
                break Event::InboundStreamInterrupted { peer_id };
            } else {
                break Event::Error(Error::CodecError(e));
            }
        }
    };

    if let Err(err) = framed.close().await {
        tracing::warn!("Error closing inbound sync stream: {}", err);
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use asynchronous_codec::{BytesMut, Encoder};
    use libp2p::{
        futures::{executor::block_on, io::Cursor},
        identity,
    };

    use super::*;
    use crate::{LocalPeerRecord, MAX_MESSAGE_SIZE, store::MemoryPeerStore};

    fn make_inbound_framed(messages: Vec<proto::Message>) -> FramedInbound<Cursor<Vec<u8>>> {
        let mut codec = quick_protobuf_codec::Codec::<proto::Message, proto::WantPeerResponse>::new(MAX_MESSAGE_SIZE);
        let mut buf = BytesMut::new();
        for msg in messages {
            codec.encode(msg, &mut buf).unwrap();
        }
        FramedInbound::new(
            Cursor::new(buf.to_vec()),
            quick_protobuf_codec::Codec::new(MAX_MESSAGE_SIZE),
        )
    }

    #[test]
    fn inbound_rejects_local_record_for_different_peer() {
        block_on(async {
            let conn_peer = PeerId::random();
            let other_kp = Arc::new(identity::Keypair::generate_ed25519());
            let mut other_rec = LocalPeerRecord::new(other_kp);
            other_rec.add_address("/ip4/127.0.0.1/tcp/9000".parse().unwrap());
            let signed_other = SignedPeerRecord::from(other_rec);

            let framed = make_inbound_framed(vec![proto::SignedPeerRecord::from(&signed_other).into()]);
            let store = MemoryPeerStore::new();
            let err = inbound_sync_task_inner(conn_peer, framed, store.clone(), Config::default())
                .await
                .unwrap_err();
            assert!(
                matches!(err, Error::InvalidSignedPeer { peer_id, .. } if peer_id == conn_peer),
                "expected InvalidSignedPeer for {conn_peer}, got {err:?}"
            );
            assert!(store.get(&signed_other.to_peer_id()).await.unwrap().is_none());
        });
    }

    #[test]
    fn inbound_accepts_same_second_update_from_authenticated_peer() {
        block_on(async {
            let kp = Arc::new(identity::Keypair::generate_ed25519());
            let peer_id = kp.public().to_peer_id();

            let mut rec1 = LocalPeerRecord::new(kp.clone());
            rec1.add_address("/ip4/127.0.0.1/tcp/9001".parse().unwrap());
            let signed1 = SignedPeerRecord::from(rec1.clone());

            // Second address added in the same second (same updated_at timestamp)
            rec1.add_address("/ip4/127.0.0.1/tcp/9002".parse().unwrap());
            let mut signed2 = SignedPeerRecord::from(rec1);
            signed2.updated_at = signed1.updated_at;
            signed2.signature = crate::PeerSignature::sign(
                &kp,
                &crate::peer_record::peer_signature_challenge(&signed2.addresses, &signed2.updated_at),
            );

            let store = MemoryPeerStore::new();
            let none_msg = proto::Message {
                payload: proto::mod_Message::OneOfpayload::None,
            };

            let framed1 = make_inbound_framed(vec![proto::SignedPeerRecord::from(&signed1).into(), none_msg.clone()]);
            inbound_sync_task_inner(peer_id, framed1, store.clone(), Config::default())
                .await
                .unwrap();
            assert_eq!(store.get(&peer_id).await.unwrap().unwrap().addresses.len(), 1);

            let framed2 = make_inbound_framed(vec![proto::SignedPeerRecord::from(&signed2).into(), none_msg]);
            inbound_sync_task_inner(peer_id, framed2, store.clone(), Config::default())
                .await
                .unwrap();
            assert_eq!(store.get(&peer_id).await.unwrap().unwrap().addresses.len(), 2);
        });
    }
}
