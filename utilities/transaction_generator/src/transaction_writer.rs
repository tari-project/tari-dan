//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io::Write, sync::mpsc, thread};

use bytes::{BufMut, Bytes, BytesMut};
use rand::rngs::OsRng;
use rayon::iter::{ParallelBridge, ParallelIterator};
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey, tari_utilities::ByteArray};
use tari_engine_types::{component::new_component_address_from_parts, instruction::Instruction};
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::{
    args,
    crypto::RistrettoPublicKeyBytes,
    models::{Amount, NonFungibleAddress},
};
use tari_transaction::Transaction;

pub fn write_transactions<W: Write>(
    num_transactions: u64,
    fee_amount: Amount,
    on_progress: &dyn Fn(usize),
    writer: &mut W,
) -> anyhow::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1000);

    thread::spawn(move || {
        (0..num_transactions).par_bridge().for_each_with(sender, |sender, _| {
            let (signer_secret_key, signer_public_key) = RistrettoPublicKey::random_keypair(&mut OsRng);

            let owner_pk = RistrettoPublicKeyBytes::from_bytes(signer_public_key.as_bytes()).unwrap();
            let owner_token = NonFungibleAddress::from_public_key(owner_pk);

            let transaction = Transaction::builder()
                .with_fee_instructions_builder(|builder| {
                    builder
                        .add_instruction(Instruction::CreateFreeTestCoins {
                            revealed_amount: Amount::new(1000),
                            output: None,
                        })
                        .put_last_instruction_output_on_workspace(b"free_coins")
                        .call_function(*ACCOUNT_TEMPLATE_ADDRESS, "create_with_bucket", args![
                            owner_token,
                            Workspace("free_coins")
                        ])
                        .call_method(
                            new_component_address_from_parts(&ACCOUNT_TEMPLATE_ADDRESS, &owner_pk.into_array().into()),
                            "pay_fee",
                            args![fee_amount],
                        )
                })
                .sign(&signer_secret_key)
                .build();

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
