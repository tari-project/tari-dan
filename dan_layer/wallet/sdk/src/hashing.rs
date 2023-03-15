//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{io, io::Write};

use digest::{Digest, FixedOutput};
use tari_bor::{encode_into, Encode};
use tari_crypto::{hash::blake2::Blake256, hash_domain, hashing::DomainSeparation};
use tari_template_lib::Hash;

hash_domain!(TariWalletHashDomain, "tari.dan.wallet_sdk", 0);

fn hasher(label: &'static str) -> TariWalletHasher {
    TariWalletHasher::new_with_label(label)
}

pub fn encrypted_value_hasher() -> TariWalletHasher {
    hasher("EncryptedValue")
}

pub fn output_mask_hasher() -> TariWalletHasher {
    hasher("OutputMask")
}

#[derive(Debug, Clone, Default)]
pub struct TariWalletHasher {
    hasher: Blake256,
}

impl TariWalletHasher {
    pub fn new_with_label(label: &'static str) -> Self {
        let mut hasher = Blake256::new();
        TariWalletHashDomain::add_domain_separation_tag(&mut hasher, label);
        Self { hasher }
    }

    pub fn update<T: Encode + ?Sized>(&mut self, data: &T) {
        encode_into(data, &mut self.hash_writer()).expect("encoding failed")
    }

    pub fn chain<T: Encode + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    #[allow(dead_code)]
    pub fn digest<T: Encode + ?Sized>(self, data: &T) -> Hash {
        self.chain(data).result()
    }

    #[allow(dead_code)]
    pub fn result(self) -> Hash {
        let hash: [u8; 32] = self.hasher.finalize().into();
        hash.into()
    }

    pub fn finalize_into(self, output: &mut digest::Output<Blake256>) {
        FixedOutput::finalize_into(self.hasher, output)
    }

    fn hash_writer(&mut self) -> impl Write + '_ {
        struct HashWriter<'a>(&'a mut Blake256);
        impl Write for HashWriter<'_> {
            fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
                self.0.update(buf);
                Ok(buf.len())
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        HashWriter(&mut self.hasher)
    }
}
