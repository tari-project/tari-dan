# tari_jellyfish

[![Crates.io](https://img.shields.io/crates/v/tari_jellyfish.svg)](https://crates.io/crates/tari_jellyfish)
[![Documentation](https://docs.rs/tari_jellyfish/badge.svg)](https://docs.rs/tari_jellyfish)

A versioned Jellyfish Merkle Tree (JMT) for Tari. Ootle validators use it to commit to their state and to the
commands in each sidechain block, and `tari_sidechain` uses it to verify inclusion proofs against those roots.

## Overview

| Item                                    | Description                                                                  |
|-----------------------------------------|------------------------------------------------------------------------------|
| `JellyfishMerkleTree`                   | Reads and updates a tree backed by a `TreeStoreReader`                       |
| `TreeStoreReader` / `TreeStoreWriter`   | Storage traits for tree nodes, implemented by the caller                     |
| `SparseMerkleProof`, `SparseMerkleProofExt` | Inclusion and exclusion proofs, and their verification                   |
| `JmtHashScheme`                         | Selects the node hashing domain and labels                                   |
| `memory_store::MemoryTreeStore`         | In-memory store for tests and tooling (`memory-store` feature)               |

## Hash schemes

Every tree and every proof verification takes an explicit `JmtHashScheme`. There is no default. A proof does not say
which scheme built it, so a verifier must take the scheme from its own authenticated context, such as the protocol
version in a signed block header.

| Scheme | Domain (name, version) | Leaf label | Internal label |
|--------|------------------------|------------|----------------|
| `V1`   | `com.tari.jmt`, 0      | `"Leaf"`   | `"Internal"`   |

A released scheme never changes. A new scheme is a new variant with its own domain version and labels.

## Verifying a proof

```rust
use tari_jellyfish::{JmtHashScheme, JmtProofVerifyError, LeafKey, SparseMerkleProofExt, TreeHash};

fn verify(
    proof: &SparseMerkleProofExt,
    root: &TreeHash,
    key: &LeafKey,
    value_hash: &TreeHash,
) -> Result<(), JmtProofVerifyError> {
    // `root` must be authenticated for this tree and version, e.g. taken from a committed block header.
    proof.verify_inclusion(JmtHashScheme::V1, root, key, value_hash)
}
```

## Features

| Feature        | Description                                                                         |
|----------------|-------------------------------------------------------------------------------------|
| `minicbor`     | Native minicbor `Encode`, `Decode` and `CborLen` impls with a compact array format  |
| `memory-store` | `memory_store::MemoryTreeStore`. Not for production use                             |

Tree, node and proof types also implement serde and borsh. Proof types are validated when decoded. Tree node types are
not, so decode them only from trusted storage.

## Version pinning

A sidechain block's `command_merkle_root` is built by the validator node and verified by `tari_sidechain`. If the two
link different `tari_jellyfish` versions, or map a protocol version to different schemes, each rejects the other's
roots.

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
