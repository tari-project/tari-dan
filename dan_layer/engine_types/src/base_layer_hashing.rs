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

use blake2::Blake2b;
use borsh::BorshSerialize;
use digest::{
    consts::{U32, U64},
    Digest,
};
use tari_common::configuration::Network;
use tari_crypto::hashing::{DomainSeparatedHasher, DomainSeparation};
use tari_hash_domains::{ConfidentialOutputHashDomain, WalletOutputEncryptionKeysDomain};

fn confidential_hasher64(network: Network, label: &'static str) -> TariBaseLayerHasher64 {
    TariBaseLayerHasher64::new_with_label::<ConfidentialOutputHashDomain>(network, label)
}

type WalletOutputEncryptionKeysDomainHasher = DomainSeparatedHasher<Blake2b<U64>, WalletOutputEncryptionKeysDomain>;

pub fn encrypted_data_hasher() -> WalletOutputEncryptionKeysDomainHasher {
    WalletOutputEncryptionKeysDomainHasher::new_with_label("")
}

pub fn ownership_proof_hasher64(network: Network) -> TariBaseLayerHasher64 {
    confidential_hasher64(network, "commitment_signature")
}

#[derive(Debug, Clone)]
pub struct TariBaseLayerHasher64 {
    hasher: Blake2b<U64>,
}

impl TariBaseLayerHasher64 {
    pub fn new_with_label<TDomain: DomainSeparation>(network: Network, label: &'static str) -> Self {
        let mut hasher = Blake2b::<U64>::new();
        TDomain::add_domain_separation_tag(&mut hasher, &format!("{}.n{}", label, network.as_byte()));
        Self { hasher }
    }

    pub fn from_digest(digest: Blake2b<U64>) -> Self {
        Self { hasher: digest }
    }

    pub fn update<T: BorshSerialize>(&mut self, data: &T) {
        BorshSerialize::serialize(data, &mut self.hash_writer())
            .expect("Incorrect implementation of BorshSerialize encountered. Implementations MUST be infallible.");
    }

    pub fn chain<T: BorshSerialize>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn chain_update<T: BorshSerialize>(self, data: &T) -> Self {
        self.chain(data)
    }

    pub fn digest<T: BorshSerialize>(self, data: &T) -> [u8; 64] {
        self.chain(data).result()
    }

    pub fn result(self) -> [u8; 64] {
        self.hasher.finalize().into()
    }

    pub fn finalize_into(self, output: &mut digest::Output<Blake2b<U64>>) {
        digest::FixedOutput::finalize_into(self.hasher, output)
    }

    fn hash_writer(&mut self) -> impl Write + '_ {
        struct HashWriter<'a>(&'a mut Blake2b<U64>);
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
#[derive(Debug, Clone)]
pub struct TariBaseLayerHasher32 {
    hasher: Blake2b<U32>,
}

impl TariBaseLayerHasher32 {
    pub fn new_with_label<TDomain: DomainSeparation>(network: Network, label: &'static str) -> Self {
        let mut hasher = Blake2b::<U32>::new();
        TDomain::add_domain_separation_tag(&mut hasher, &format!("{}.n{}", label, network.as_byte()));
        Self { hasher }
    }

    pub fn from_digest(digest: Blake2b<U32>) -> Self {
        Self { hasher: digest }
    }

    pub fn update<T: BorshSerialize>(&mut self, data: &T) {
        BorshSerialize::serialize(data, &mut self.hash_writer())
            .expect("Incorrect implementation of BorshSerialize encountered. Implementations MUST be infallible.");
    }

    pub fn chain<T: BorshSerialize>(mut self, data: &T) -> Self {
        self.update(data);
        self
    }

    pub fn chain_update<T: BorshSerialize>(self, data: &T) -> Self {
        self.chain(data)
    }

    pub fn digest<T: BorshSerialize>(self, data: &T) -> [u8; 32] {
        self.chain(data).result()
    }

    pub fn result(self) -> [u8; 32] {
        self.hasher.finalize().into()
    }

    pub fn finalize_into(self, output: &mut digest::Output<Blake2b<U32>>) {
        digest::FixedOutput::finalize_into(self.hasher, output)
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
