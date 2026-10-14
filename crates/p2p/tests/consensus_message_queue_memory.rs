//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Heap held by the inbound consensus message queue, against the bytes it charges to its budget.

mod support;

use std::{collections::HashMap, sync::atomic::Ordering};

use libp2p::futures::executor::block_on;
use prost::Message;
use support::{OUTSTANDING, PEAK, SERIAL};
use tari_networking::{MessagingMode, PeerId, message_queue};
use tari_ootle_p2p::proto;
use tari_swarm::messaging::{BytesCodec, Codec, MAX_MESSAGE_SIZE};

/// A length-prefixed frame carrying `message`, as the direct messaging protocol puts it on the wire.
fn frame(message: &proto::consensus::HotStuffMessage) -> Vec<u8> {
    let body = message.encode_to_vec();
    assert!(
        body.len() <= MAX_MESSAGE_SIZE,
        "{} byte frame is over the limit",
        body.len()
    );
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    frame
}

/// A proposal whose block lists as many empty commands as fit in one frame.
fn proposal_of_empty_commands() -> proto::consensus::HotStuffMessage {
    let message = |commands| proto::consensus::HotStuffMessage {
        message: Some(proto::consensus::hot_stuff_message::Message::Proposal(
            proto::consensus::ProposalMessage {
                block: Some(proto::consensus::Block {
                    commands,
                    ..Default::default()
                }),
                foreign_proposals: vec![],
            },
        )),
    };
    let overhead = message(vec![]).encoded_len();
    // Each empty command encodes to its field tag and a zero length.
    let commands = vec![proto::consensus::Command::default(); (MAX_MESSAGE_SIZE - overhead - 16) / 2];
    message(commands)
}

/// Heap measured while a frame is received into the inbound queue.
struct Received {
    /// Bytes the queue charged for the message.
    charged: usize,
    /// Heap the queued message holds.
    held: usize,
    /// The most heap taken at once while receiving it, beyond the frame itself.
    peak: usize,
}

/// Receives `frame` from a peer as the networking worker does, reading it with the direct messaging protocol's codec
/// and admitting it to the inbound queue.
fn receive_into_queue(frame: Vec<u8>) -> Received {
    let (tx, rx) = message_queue(16, 128 * 1024 * 1024);
    let mode = MessagingMode::Enabled {
        tx_messages: tx.clone(),
        tx_gossip_messages_by_topic: HashMap::new(),
    };

    let baseline = OUTSTANDING.load(Ordering::Relaxed);
    PEAK.store(baseline, Ordering::Relaxed);
    let (_, message) = block_on(BytesCodec.decode_from(&mut frame.as_slice())).unwrap();
    drop(frame);
    mode.send_message(PeerId::random(), message).unwrap();
    let held = OUTSTANDING.load(Ordering::Relaxed).saturating_sub(baseline);
    let peak = PEAK.load(Ordering::Relaxed).saturating_sub(baseline);
    let charged = tx.queued_bytes();

    drop(rx);
    Received { charged, held, peak }
}

/// The inbound queue's byte budget is what bounds the memory its queued messages hold, so receiving a message must
/// cost about the bytes it arrived as, and the queued message must hold no more heap than the queue charges for it,
/// however its payload is laid out.
#[test]
fn a_queued_message_holds_no_more_heap_than_the_queue_charges_for_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let frame = frame(&proposal_of_empty_commands());
    let wire_len = frame.len();

    let Received { charged, held, peak } = receive_into_queue(frame);

    assert!(
        charged > 0 && charged <= wire_len,
        "the queue charged {charged} bytes for a {wire_len} byte frame"
    );
    // Room for the queue slot and the channel's own bookkeeping beside the payload.
    let slack = 64 * 1024;
    assert!(
        held <= charged + slack,
        "a message charged {charged} bytes holds {held} bytes of heap in the queue"
    );
    assert!(
        peak <= wire_len + slack,
        "receiving a {wire_len} byte frame peaked at {peak} bytes of heap"
    );
}
