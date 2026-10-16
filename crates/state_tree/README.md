# tari_state_tree

[![Crates.io](https://img.shields.io/crates/v/tari_state_tree.svg)](https://crates.io/crates/tari_state_tree)
[![Documentation](https://docs.rs/tari_state_tree/badge.svg)](https://docs.rs/tari_state_tree)

The Merkle trees behind Tari Ootle's committed state. Each shard keeps a Jellyfish Merkle tree of its substate
versions. A shard group's per-shard roots combine into the state root that every block header commits, and each
epoch's validator set is a tree with one leaf per shard group. Proofs against these roots let a client check
substates and committees without trusting the node that served them.

## Key types

| Type                                               | Description                                                                                         |
|----------------------------------------------------|-----------------------------------------------------------------------------------------------------|
| `StateTree` / `SpreadPrefixStateTree`              | A shard's Jellyfish Merkle tree of substate versions over a pluggable node store                    |
| `ShardGroupRootTree`                               | The tree over a shard group's per-shard roots, whose root is a block header's `state_merkle_root`   |
| `SubstateValueProof`                               | Proves a substate version's value, or its absence, under a shard group root                         |
| `SubstateDownProof`                                | Proves a substate version was committed and has since gone down, against two roots                  |
| `ValidatorSetTree`                                 | An epoch's validator set, one leaf per shard group committee; its root gives the `ValidatorSetHash` |
| `ValidatorSetCommitteeProof`                       | Proves one shard group's committee is in the validator set a `ValidatorSetHash` commits to          |
| `RootProofTree` / `compute_merkle_root_for_hashes` | Merkle roots and proofs over a set of hashes, such as a block's commands                            |
| `MemoryTreeStore` / `StagedTreeStore`              | An in-memory node store, and a store that stages pending versions over a readable one               |

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
