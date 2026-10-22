# tari_sidechain

[![Crates.io](https://img.shields.io/crates/v/tari_sidechain.svg)](https://crates.io/crates/tari_sidechain)
[![Documentation](https://docs.rs/tari_sidechain/badge.svg)](https://docs.rs/tari_sidechain)

Types and verification for Tari Ootle sidechain proofs, shared by Ootle and the Minotari base layer:

- Commit proofs, which prove that a command (such as an end of epoch) was committed by an Ootle validator committee.
- Burn claim proofs, which Minotari wallets build for funds burnt on the base layer and Ootle verifies to mint them.

Apart from `tari_jellyfish`, it depends on no Minotari or Ootle crates, so either side can depend on it. It does not
depend on `tari_hashing`, so Minotari can publish pre-releases of that crate without breaking its own dependency on
this one.

## Key types

| Type                         | Description                                                                              |
|------------------------------|------------------------------------------------------------------------------------------|
| `CommandCommitProof<C>`      | A command, the proof that its block committed, and its inclusion in that block           |
| `SidechainBlockCommitProof`  | A block header and the 3-chain of quorum certificates that commits it                    |
| `SidechainBlockHeader`       | The header fields a commit proof binds, including `network`, `epoch` and `shard_group`   |
| `QuorumCertificate`          | A committee's signatures over a block, its signers named by a `SignerBitmap`             |
| `SigningCommittee`           | The committee a proof is checked against, in ascending order of public key               |
| `Command`, `EndEpochAtom`    | Commands a sidechain block can contain                                                   |
| `BurnClaimProof`             | Proves ownership of a base layer burn output and its inclusion in a mined block          |
| `BurnOutputProof`            | The burn output and its two-level MMR inclusion proof against `block_output_mr`          |
| `FixedHash`                  | A 32-byte hash, encoded the same as the base layer's `FixedHash`                         |

The `hashing` module holds the consensus hashers (domain `com.tari.consensus`) shared by validators, the base layer
and clients.

## Validating a commit proof

The caller supplies the committee for the proof's epoch and shard group as a `SigningCommittee`: its members'
32-byte compressed public keys and voting power, in ascending lexicographic order of key. Each quorum certificate names
its signers by their indices in that order. `quorum_threshold` must be positive.

```rust
use tari_sidechain::{CommandCommitProof, EndEpochAtom, SidechainProofValidationError, SigningCommittee};

fn validate(
    proof: &CommandCommitProof<EndEpochAtom>,
    network: u8,
    quorum_threshold: u64,
    committee: &dyn SigningCommittee,
) -> Result<(), SidechainProofValidationError> {
    // `quorum_threshold` and `committee` must describe the committee of `proof.epoch()` and `proof.shard_group()`.
    proof.validate_committed(network, quorum_threshold, committee)
}
```

Validation checks the proof's structure, the network, each signer bitmap and quorum before verifying any signature,
so an invalid proof costs little to reject.

## Stability

Hash domains, labels and the serde and borsh encodings of proof types are consensus-critical and change only at a
network reset. `tests/golden_vectors.rs` pins them.

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
