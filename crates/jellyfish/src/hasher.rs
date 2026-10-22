// Copyright 2024 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

//! The domain-separated Borsh hasher behind the JMT and `tari_sidechain` consensus hashes.
//!
//! This is a copy of `tari_hashing::DomainSeparatedBorshHasher` (6.2.0), kept here so that neither published crate
//! depends on `tari_hashing`. Minotari pre-releases bump `tari_hashing` to a pre-release version, which a `^6.2.0`
//! requirement never matches, so a dependency on it blocks Minotari from publishing anything that also uses
//! `tari_sidechain`. The output is byte-identical: the known-answer tests in `tree.rs` and `tari_sidechain::hashing`
//! pin it.

use std::{io, io::Write, marker::PhantomData};

use blake2::Blake2b;
use borsh::BorshSerialize;
use digest::{Digest, consts::U32};
use tari_crypto::hashing::DomainSeparation;

/// A domain-separated hasher that feeds the Borsh encoding of its input to the digest, so hashing is canonical.
///
/// This assumes that any input type supports `BorshSerialize` canonically; that is, two different values of the same
/// type must serialize distinctly.
pub struct DomainSeparatedBorshHasher<M, D> {
    writer: WriteHashWrapper<D>,
    _m: PhantomData<M>,
}

impl<D: Digest + Default, M: DomainSeparation> DomainSeparatedBorshHasher<M, D> {
    pub fn new_with_label(label: &str) -> Self {
        let mut digest = D::default();
        M::add_domain_separation_tag(&mut digest, label);
        Self {
            writer: WriteHashWrapper(digest),
            _m: PhantomData,
        }
    }

    pub fn finalize(self) -> digest::Output<D> {
        self.writer.0.finalize()
    }

    pub fn finalize_into_array<const I: usize>(self) -> [u8; I]
    where [u8; I]: From<digest::Output<D>> {
        self.writer.0.finalize().into()
    }

    /// Update the hasher using the Borsh encoding of the input, which is assumed to be canonical.
    pub fn update_consensus_encode<T: BorshSerialize + ?Sized>(&mut self, data: &T) {
        BorshSerialize::serialize(data, &mut self.writer)
            .expect("Incorrect implementation of BorshSerialize encountered. Implementations MUST be infallible.");
    }

    pub fn chain<T: BorshSerialize + ?Sized>(mut self, data: &T) -> Self {
        self.update_consensus_encode(data);
        self
    }
}

/// A [`DomainSeparatedBorshHasher`] over Blake2b with an output of `OutSize` bytes.
pub type TariDomainHasher<M, OutSize> = DomainSeparatedBorshHasher<M, Blake2b<OutSize>>;

/// Returns a 32-byte [`TariDomainHasher`] for the domain `M` and the given label.
pub fn tari_hasher32<M: DomainSeparation>(label: &'static str) -> TariDomainHasher<M, U32> {
    TariDomainHasher::<M, U32>::new_with_label(label)
}

/// Wraps a digest and implements `Write`, so Borsh can serialize straight into it.
///
/// Never use `DomainSeparatedHasher` with this, since it can handle length prepending inconsistently.
#[derive(Clone)]
struct WriteHashWrapper<D>(D);

impl<D: Digest> Write for WriteHashWrapper<D> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use digest::consts::U32;
    use tari_crypto::hash_domain;

    use super::*;

    #[derive(BorshSerialize)]
    struct TestStruct {
        a: u64,
        b: u64,
    }

    hash_domain!(TestHashDomain, "com.tari.test.test_hash", 0);

    #[test]
    fn it_hashes_the_borsh_encoding_after_the_domain_tag() {
        let s1 = TestStruct { a: 1, b: 2 };
        let s2 = TestStruct { a: 3, b: 4 };
        let mut buf = Vec::new();
        BorshSerialize::serialize(&s1, &mut buf).unwrap();
        BorshSerialize::serialize(&s2, &mut buf).unwrap();

        let mut expected = Blake2b::<U32>::default();
        TestHashDomain::add_domain_separation_tag(&mut expected, "foo");
        let expected = expected.chain_update(&buf).finalize();

        let hash = tari_hasher32::<TestHashDomain>("foo").chain(&s1).chain(&s2).finalize();
        assert_eq!(hash, expected);
    }

    #[test]
    fn labels_yield_distinct_hashes() {
        let input = [1u8; 32];
        let h1 = tari_hasher32::<TestHashDomain>("label1").chain(&input).finalize();
        let h2 = tari_hasher32::<TestHashDomain>("label2").chain(&input).finalize();
        assert_ne!(h1, h2);
    }
}
