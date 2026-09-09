//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! `read_transactions` yields the file's entries from `skip` onwards, in file order.

use std::io::{Cursor, Seek, SeekFrom};

use tari_ootle_transaction::{Epoch, Network, Transaction, TransactionId};
use transaction_generator::{read_transactions, transaction_builders::free_coins, write_transactions};

const NUM_TRANSACTIONS: u64 = 8;

/// A transaction file holding [`NUM_TRANSACTIONS`] entries, positioned at its start.
fn write_file() -> Cursor<Vec<u8>> {
    let mut buf = Cursor::new(Vec::new());
    write_transactions(
        NUM_TRANSACTIONS,
        Box::new(free_coins::builder(Network::LocalNet, Epoch(1))),
        &|_| {},
        &mut buf,
    )
    .unwrap();
    buf.seek(SeekFrom::Start(0)).unwrap();
    buf
}

fn read_ids(file: Cursor<Vec<u8>>, skip: u64) -> Vec<TransactionId> {
    read_transactions(file, skip)
        .unwrap()
        .into_iter()
        .map(|t: Transaction| t.calculate_id())
        .collect()
}

#[test]
fn reads_every_transaction_when_nothing_is_skipped() {
    let ids = read_ids(write_file(), 0);
    assert_eq!(ids.len(), NUM_TRANSACTIONS as usize);
}

#[test]
fn reads_the_tail_of_the_file_after_a_skip() {
    const SKIP: u64 = 5;
    // The same bytes read twice: each transaction is sealed with a fresh random key, so a second
    // call to `write_file` would produce a different set to compare against.
    let file = write_file();
    let all = read_ids(file.clone(), 0);
    let tail = read_ids(file, SKIP);

    assert_eq!(tail, all[SKIP as usize..]);
}

#[test]
fn skipping_every_transaction_yields_nothing() {
    assert!(read_ids(write_file(), NUM_TRANSACTIONS).is_empty());
}
