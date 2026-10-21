// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! The consensus hashers of the Ootle sidechain. Validators, the base layer and every client that verifies a commit
//! proof hash with these, so a domain, version or label changes only at a network reset.

use digest::consts::U32;
use tari_crypto::hash_domain;
use tari_hashing::layer2::TariDomainHasher;

hash_domain!(TariDanConsensusHashDomain, "com.tari.consensus", 0);

pub type TariConsensusHasher = TariDomainHasher<TariDanConsensusHashDomain, U32>;

pub fn tari_consensus_hasher(label: &'static str) -> TariConsensusHasher {
    TariConsensusHasher::new_with_label(label)
}

/// Hashes a block header's fields into the header hash.
pub fn block_header_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("BlockHeader")
}

/// Hashes a block's parent id and header hash into its block id.
pub fn block_id_hasher() -> TariConsensusHasher {
    tari_consensus_hasher("BlockId")
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

    /// Known-answer digests. Every hasher but `block_header_hasher` and `block_id_hasher` matches `tari_hashing`
    /// 6.2.0's `tests/vectors.json`, where those hashers lived.
    #[test]
    fn hashers_match_known_answer_digests() {
        let input = input();
        let cases = [
            (
                block_header_hasher(),
                "349bf77f7738a5cd00f4ab28dfc200bd878b67710fa29a9d996829610488c4d6",
            ),
            (
                block_id_hasher(),
                "f2cb0fc426295a160acbc5557ba9af8951c99d68c18a4177d966b60789592332",
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
