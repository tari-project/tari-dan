// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! The consensus hashers of the Ootle sidechain. Validators, the base layer and every client that verifies a commit
//! proof hash with these, so a domain, version or label never changes once released.

use digest::consts::U32;
use tari_crypto::hash_domain;
use tari_hashing::layer2::TariDomainHasher;

hash_domain!(TariDanConsensusHashDomain, "com.tari.consensus", 0);

pub type TariConsensusHasher = TariDomainHasher<TariDanConsensusHashDomain, U32>;

pub fn tari_consensus_hasher(label: &'static str) -> TariConsensusHasher {
    TariConsensusHasher::new_with_label(label)
}

pub fn block_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("Block")
}

pub fn block_metadata_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("BlockMetadata")
}

pub fn command_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("Command")
}

pub fn proposal_vote_signature_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("VoteSignature")
}

pub fn timeout_vote_signature_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("TimeoutVoteSignature")
}

#[cfg(test)]
mod tests {
    use tari_crypto::hashing::DomainSeparation;
    use tari_utilities::hex::to_hex;

    use super::*;

    /// The known-answer input of `tari_hashing`'s `tests/vectors.json`: the bytes 0 to 31.
    fn input() -> [u8; 32] {
        std::array::from_fn(|i| u8::try_from(i).unwrap())
    }

    #[test]
    fn the_domain_matches_tari_hashing_vectors() {
        assert_eq!(TariDanConsensusHashDomain::domain(), "com.tari.consensus");
        assert_eq!(TariDanConsensusHashDomain::version(), 0);
    }

    /// Digests pinned in `tari_hashing` 6.2.0's `tests/vectors.json`, where these hashers lived.
    #[test]
    fn hashers_match_tari_hashing_vectors() {
        let input = input();
        let cases = [
            (
                block_hasher(),
                "f05946eebffad9c503cd20dccdd138d98235293b981ba4b17b7ca356da993509",
            ),
            (
                block_metadata_hasher(),
                "a37f79b8f3a1ec653b3aae1c71cc319f0e024f7164071300b6ffccfa86e7d156",
            ),
            (
                command_hasher(),
                "272f1329b678e6373ba77b65c5d585a770cc958b7f954dbbafc5cfdf8391cb3b",
            ),
            (
                proposal_vote_signature_hasher(),
                "87478974a6118e9d011e69bcb16c0af856f0f90150db8df467c8c4abea20fbb9",
            ),
            (
                timeout_vote_signature_hasher(),
                "50896752a994b3e290869e6726efe582cfa3094486839d5e55f94b0441a20571",
            ),
        ];
        for (i, (hasher, expected)) in cases.into_iter().enumerate() {
            assert_eq!(to_hex(&hasher.chain(&input).finalize()), expected, "case {i}");
        }
    }
}
