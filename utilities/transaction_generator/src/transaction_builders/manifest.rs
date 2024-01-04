//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{collections::HashMap, fs, path::Path};

use tari_crypto::ristretto::RistrettoSecretKey;
use tari_transaction::Transaction;
use tari_transaction_manifest::ManifestValue;

use crate::BoxedTransactionBuilder;

pub fn builder<P: AsRef<Path>>(
    signer_secret_key: RistrettoSecretKey,
    manifest: P,
    globals: HashMap<String, ManifestValue>,
) -> anyhow::Result<BoxedTransactionBuilder> {
    let contents = fs::read_to_string(manifest).unwrap();
    let instructions = tari_transaction_manifest::parse_manifest(&contents, globals)?;
    Ok(Box::new(move |_| {
        Transaction::builder()
            .with_fee_instructions_builder(|builder| builder.with_instructions(instructions.fee_instructions.clone()))
            .with_instructions(instructions.instructions.clone())
            .sign(&signer_secret_key)
            .build()
    }))
}
