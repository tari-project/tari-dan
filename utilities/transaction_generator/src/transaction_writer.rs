//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io::Write, sync::mpsc, thread};

use bytes::{BufMut, Bytes, BytesMut};
use rayon::iter::{ParallelBridge, ParallelIterator};

use crate::BoxedTransactionBuilder;

pub fn write_transactions<W: Write>(
    num_transactions: u64,
    builder: BoxedTransactionBuilder,
    on_progress: &dyn Fn(usize),
    writer: &mut W,
) -> anyhow::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1000);

    thread::spawn(move || {
        (0..num_transactions).par_bridge().for_each_with(sender, |sender, i| {
            let transaction = builder(i);
            let buf = bincode::serde::encode_to_vec(&transaction, bincode::config::standard()).unwrap();
            let buf = Bytes::from(buf);
            let output = BytesMut::with_capacity(buf.len() + 2);
            let len = (u16::try_from(buf.len()).unwrap()).to_le_bytes();
            let mut writer = output.writer();
            writer.write_all(&len).unwrap();
            writer.write_all(&buf).unwrap();
            sender.send(writer.into_inner().freeze()).unwrap();
        });
    });

    let len_bytes = num_transactions.to_le_bytes();
    bincode::serde::encode_into_std_write(len_bytes, writer, bincode::config::standard()).unwrap();
    let mut count = 0;
    while let Ok(buf) = receiver.recv() {
        writer.write_all(&buf)?;
        count += 1;
        if count % 10000 == 0 {
            on_progress(count);
        }
    }

    Ok(())
}
