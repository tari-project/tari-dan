//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! The reader stops at the file's last entry when `skip` is non-zero.
//!
//! An over-read fails inside the reader's own thread, after it has sent every transaction the
//! caller is owed, so the received count is the same either way and only the panic tells them
//! apart — hence the panic hook. Hooks are process-wide, so this test has a binary to itself.

use std::{
    io::{Cursor, Seek, SeekFrom},
    panic,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use tari_ootle_transaction::{Epoch, Network};
use transaction_generator::{read_transactions, transaction_builders::free_coins, write_transactions};

const NUM_TRANSACTIONS: u64 = 8;
const SKIP: u64 = 5;

#[test]
fn reading_a_skipped_file_to_its_end_does_not_over_read() {
    let mut file = Cursor::new(Vec::new());
    write_transactions(
        NUM_TRANSACTIONS,
        Box::new(free_coins::builder(Network::LocalNet, Epoch(1))),
        &|_| {},
        &mut file,
    )
    .unwrap();
    file.seek(SeekFrom::Start(0)).unwrap();

    let panicked = Arc::new(AtomicBool::new(false));
    let previous = panic::take_hook();
    panic::set_hook({
        let panicked = Arc::clone(&panicked);
        Box::new(move |info| {
            panicked.store(true, Ordering::SeqCst);
            eprintln!("reader panicked: {info}");
        })
    });

    let transactions: Vec<_> = read_transactions(file, SKIP).unwrap().into_iter().collect();

    panic::set_hook(previous);

    assert_eq!(transactions.len(), (NUM_TRANSACTIONS - SKIP) as usize);
    assert!(
        !panicked.load(Ordering::SeqCst),
        "the reader read past the end of the file"
    );
}
