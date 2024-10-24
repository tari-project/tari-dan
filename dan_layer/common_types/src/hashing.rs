//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-clause

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

use blake2::{digest::consts::U32, Blake2b};
use tari_crypto::{hash_domain, hashing::DomainSeparatedHasher};
use tari_hashing::ValidatorNodeMerkleHashDomain;
use tari_mmr::{BalancedBinaryMerkleProof, BalancedBinaryMerkleTree, MergedBalancedBinaryMerkleProof};

use crate::hasher::{tari_hasher, TariHasher};

hash_domain!(TariDanConsensusHashDomain, "com.tari.dan.consensus", 0);

pub fn block_hasher() -> TariHasher {
    dan_hasher("Block")
}

pub fn quorum_certificate_hasher() -> TariHasher {
    dan_hasher("QuorumCertificate")
}

pub fn pledge_hasher() -> TariHasher {
    dan_hasher("Pledges")
}

pub fn vote_hasher() -> TariHasher {
    dan_hasher("Vote")
}

pub fn vote_signature_hasher() -> TariHasher {
    dan_hasher("VoteSignature")
}

fn dan_hasher(label: &'static str) -> TariHasher {
    tari_hasher::<TariDanConsensusHashDomain>(label)
}

pub type ValidatorNodeBmtHasherBlake2b = DomainSeparatedHasher<Blake2b<U32>, ValidatorNodeMerkleHashDomain>;
pub type ValidatorNodeBalancedMerkleTree = BalancedBinaryMerkleTree<ValidatorNodeBmtHasherBlake2b>;
pub type ValidatorNodeMerkleProof = BalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake2b>;
pub type MergedValidatorNodeMerkleProof = MergedBalancedBinaryMerkleProof<ValidatorNodeBmtHasherBlake2b>;
