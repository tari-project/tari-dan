//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    io::{Read, Seek, SeekFrom},
    sync::mpsc,
    thread,
};

use tari_ootle_transaction::Transaction;

pub fn read_transactions<R: Read + Seek + Send + 'static>(
    mut reader: R,
    skip: u64,
) -> anyhow::Result<mpsc::Receiver<Transaction>> {
    let (sender, receiver) = mpsc::sync_channel(1000);
    thread::spawn(move || {
        let mut remaining = read_number_of_transactions(&mut reader).unwrap();
        let mut skip_remaining = skip;

        while remaining > 0 {
            let mut len_bytes = [0u8; 2];
            reader.read_exact(&mut len_bytes).unwrap();
            let len = usize::from(u16::from_le_bytes(len_bytes));
            remaining -= 1;

            if skip_remaining > 0 {
                skip_remaining -= 1;
                reader.seek(SeekFrom::Current(len as i64)).unwrap();
                continue;
            }

            let mut buf = vec![0u8; len];
            reader.read_exact(&mut buf).unwrap();
            let transaction: Transaction = tari_bor::decode(&buf).unwrap();
            if sender.send(transaction).is_err() {
                // Receiver has closed the channel, so we're done
                break;
            }
        }
    });
    Ok(receiver)
}

pub fn read_number_of_transactions<R: Read>(reader: &mut R) -> anyhow::Result<u64> {
    let mut len_bytes = [0u8; 8];
    reader.read_exact(&mut len_bytes).unwrap();
    Ok(u64::from_le_bytes(len_bytes))
}
