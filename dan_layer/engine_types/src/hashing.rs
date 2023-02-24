//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{io, io::Write};

use digest::Digest;
use tari_bor::{encode_into, Encode};
use tari_crypto::{hash::blake2::Blake256, hash_domain, hashing::DomainSeparation};
use tari_template_lib::Hash;

hash_domain!(TariEngineHashDomain, "tari.dan.engine", 0);

// TODO: use an enum for labels
pub fn hasher(label: &'static str) -> TariEngineHasher {
    TariEngineHasher::new_with_label(label)
}

#[derive(Debug, Clone)]
pub struct TariEngineHasher {
    hasher: Blake256,
}

impl TariEngineHasher {
    pub fn new_with_label(label: &'static str) -> Self {
        let mut hasher = Blake256::new();
        TariEngineHashDomain::add_domain_separation_tag(&mut hasher, label);
        Self { hasher }
    }

    pub fn update<T: Encode + ?Sized>(&mut self, data: &T) {
        // Borsh encoding does not make any contract to say that if the writer is infallible (as it is here) then
        // encoding in infallible. However this should be the case. Since it is very unergonomic to return an
        // error in hash chain functions, and therefore all usages of the hasher, we assume all types implement
        // infallible encoding.
        encode_into(data, &mut self.hash_writer()).expect("encoding failed")
    }

    pub fn chain<T: Encode + ?Sized>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn digest<T: Encode + ?Sized>(self, data: &T) -> Hash {
        self.chain(data).result()
    }

    pub fn result(self) -> Hash {
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
