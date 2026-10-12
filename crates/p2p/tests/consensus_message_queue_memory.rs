//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Heap held by the inbound consensus message queue, against the bytes it charges to its budget.
//!
//! Kept in a test binary of its own because it installs a counting global allocator, and any other
//! test running alongside it would be counted too. The tests here take [`SERIAL`] for the same reason.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    collections::HashMap,
    sync::{
        Mutex,
        atomic::{AtomicUsize, Ordering},
    },
};

use libp2p::futures::executor::block_on;
use prost::Message;
use tari_networking::{MessagingMode, PeerId, message_queue};
use tari_ootle_p2p::proto;
use tari_swarm::messaging::{BytesCodec, Codec};

static SERIAL: Mutex<()> = Mutex::new(());

/// The most bytes the direct messaging protocol accepts in one message.
const MAX_MESSAGE_SIZE: usize = 4 * 1024 * 1024;

struct Counting;

static OUTSTANDING: AtomicUsize = AtomicUsize::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            OUTSTANDING.fetch_add(layout.size(), Ordering::Relaxed);
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        OUTSTANDING.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) };
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                OUTSTANDING.fetch_add(new_size - layout.size(), Ordering::Relaxed);
            } else {
                OUTSTANDING.fetch_sub(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

#[global_allocator]
static ALLOC: Counting = Counting;

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

/// Receives `frame` from a peer as the networking worker does, reading it with the direct messaging protocol's codec
/// and admitting it to the inbound queue, and returns the bytes the inbound queue
/// charged for it and the heap held while it sits in the queue.
fn receive_into_queue(frame: Vec<u8>) -> (usize, usize) {
    let (tx, rx) = message_queue(16, 128 * 1024 * 1024);
    let mode = MessagingMode::Enabled {
        tx_messages: tx.clone(),
        tx_gossip_messages_by_topic: HashMap::new(),
    };

    let baseline = OUTSTANDING.load(Ordering::Relaxed);
    let (_, message) = block_on(BytesCodec.decode_from(&mut frame.as_slice())).unwrap();
    drop(frame);
    mode.send_message(PeerId::random(), message).unwrap();
    let held = OUTSTANDING.load(Ordering::Relaxed).saturating_sub(baseline);
    let charged = tx.queued_bytes();

    drop(rx);
    (charged, held)
}

/// The inbound queue's byte budget is what bounds the memory its queued messages hold, so a queued message must
/// hold no more heap than the queue charges for it, however its payload is laid out.
#[test]
fn a_queued_message_holds_no_more_heap_than_the_queue_charges_for_it() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let frame = frame(&proposal_of_empty_commands());
    let wire_len = frame.len();

    let (charged, held) = receive_into_queue(frame);

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
}
