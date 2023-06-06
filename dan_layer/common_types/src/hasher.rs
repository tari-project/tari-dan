//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::{io, io::Write};

use digest::Digest;
use serde::Serialize;
use tari_bor::encode_into;
use tari_common_types::types::FixedHash;
use tari_crypto::{hash::blake2::Blake256, hashing::DomainSeparation};

pub fn tari_hasher<D: DomainSeparation>(label: &'static str) -> TariHasher {
    TariHasher::new_with_label::<D>(label)
}

#[derive(Debug, Clone)]
pub struct TariHasher {
    hasher: Blake256,
}

impl TariHasher {
    pub fn new_with_label<D: DomainSeparation>(label: &'static str) -> Self {
        let mut hasher = Blake256::new();
        D::add_domain_separation_tag(&mut hasher, label);
        Self { hasher }
    }

    pub fn update<T: Serialize + ?Sized>(&mut self, data: &T) {
        // Binary encoding does not make any contract to say that if the writer is infallible (as it is here) then
        // encoding in infallible. However this should be the case. Since it is very unergonomic to return an
        // error in hash chain functions, and therefore all usages of the hasher, we assume all types implement
        // infallible encoding.
        encode_into(data, &mut self.hash_writer()).expect("encoding failed")
    }

    pub fn chain<T: Serialize + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn digest<T: Serialize + ?Sized>(self, data: &T) -> FixedHash {
        self.chain(data).result()
    }

    pub fn result(self) -> FixedHash {
        let hash: [u8; 32] = self.hasher.finalize().into();
        hash.into()
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
