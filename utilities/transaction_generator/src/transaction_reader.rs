//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io::Read, sync::mpsc, thread};

use tari_transaction::Transaction;

pub fn read_transactions<R: Read + Send + 'static>(mut reader: R) -> anyhow::Result<mpsc::Receiver<Transaction>> {
    let (sender, receiver) = mpsc::sync_channel(1000);
    thread::spawn(move || {
        let mut remaining = read_number_of_transactions(&mut reader).unwrap();

        while remaining > 0 {
            let mut len_bytes = [0u8; 2];
            reader.read_exact(&mut len_bytes).unwrap();
            let len = u16::from_le_bytes(len_bytes) as u64;
            let mut limited_reader = (&mut reader).take(len);
            let transaction: Transaction =
                bincode::serde::decode_from_std_read(&mut limited_reader, bincode::config::standard()).unwrap();
            sender.send(transaction).unwrap();
            remaining -= 1;
        }
    });
    Ok(receiver)
}

pub fn read_number_of_transactions<R: Read>(reader: &mut R) -> anyhow::Result<u64> {
    let mut len_bytes = [0u8; 8];
    reader.read_exact(&mut len_bytes).unwrap();
    Ok(u64::from_le_bytes(len_bytes))
}
