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

use blake2::Blake2b;
use digest::consts::{U32, U64};
use tari_common::configuration::Network;
use tari_crypto::hashing::DomainSeparatedHasher;
use tari_hashing::{ConfidentialOutputHashDomain, DomainSeparatedBorshHasher, WalletOutputEncryptionKeysDomain};

pub type TariBaseLayerHasher64<M> = DomainSeparatedBorshHasher<M, Blake2b<U64>>;
pub type TariBaseLayerHasher32<M> = DomainSeparatedBorshHasher<M, Blake2b<U32>>;
fn confidential_hasher64(network: Network, label: &'static str) -> TariBaseLayerHasher64<ConfidentialOutputHashDomain> {
    DomainSeparatedBorshHasher::<_, Blake2b<U64>>::new_with_label(&format!("{}.n{}", label, network.as_byte()))
}

type WalletOutputEncryptionKeysDomainHasher = DomainSeparatedHasher<Blake2b<U64>, WalletOutputEncryptionKeysDomain>;

pub fn encrypted_data_hasher() -> WalletOutputEncryptionKeysDomainHasher {
    WalletOutputEncryptionKeysDomainHasher::new_with_label("")
}

pub fn ownership_proof_hasher64(network: Network) -> TariBaseLayerHasher64<ConfidentialOutputHashDomain> {
    confidential_hasher64(network, "commitment_signature")
}
