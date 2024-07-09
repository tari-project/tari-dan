//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
use std::{io, io::Write};

use blake2::{
    digest::{consts::U32, Digest},
    Blake2b,
};
use serde::Serialize;
use tari_bor::encode_into;
use tari_common_types::types::FixedHash;
use tari_crypto::hashing::DomainSeparation;

/// Create a new `TariHasher` using a given domain-separated hasher and label.
/// This is just a wrapper,
pub fn tari_hasher<D: DomainSeparation>(label: &'static str) -> TariHasher {
    TariHasher::new_with_label::<D>(label)
}

/// A domain-separated hasher that uses CBOR internally to ensure hashing is canonical.
///
/// The hasher produces 32 bytes of output using the `Blake2b` hash function.
///
/// This assumes that any input type supports `Serialize` canonically; that is, two different values of the same type
/// must serialize distinctly.
#[derive(Debug, Clone)]
pub struct TariHasher {
    hasher: Blake2b<U32>,
}

impl TariHasher {
    pub fn new_with_label<D: DomainSeparation>(label: &'static str) -> Self {
        let mut hasher = Blake2b::<U32>::new();
        D::add_domain_separation_tag(&mut hasher, label);
        Self { hasher }
    }

    pub fn update<T: Serialize + ?Sized>(&mut self, data: &T) {
        // Update the hasher using the CBOR encoding of the input, which is assumed to be canonical.
        //
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
        self.finalize_into_array().into()
    }

    pub fn finalize_into_array(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }

    fn hash_writer(&mut self) -> impl Write + '_ {
        struct HashWriter<'a>(&'a mut Blake2b<U32>);
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
