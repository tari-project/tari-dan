# tari_ootle_storage

[![Crates.io](https://img.shields.io/crates/v/tari_ootle_storage.svg)](https://crates.io/crates/tari_ootle_storage)
[![Documentation](https://docs.rs/tari_ootle_storage/badge.svg)](https://docs.rs/tari_ootle_storage)

Consensus models, storage traits and proof verification for Tari Ootle. A validator node keeps its consensus state
behind the `StateStore` traits defined here. The same models carry the proofs that let a client verify blocks,
substates and validator set handovers without trusting the node that served them.

## Modules

| Module             | Contents                                                                                                 |
|--------------------|----------------------------------------------------------------------------------------------------------|
| `consensus_models` | Blocks, transactions, substates, pledges and certificates as consensus stores them, plus the proof types |
| `global`           | The global database: epochs, validator nodes, block headers and metadata, behind `GlobalDbAdapter`       |

## Key types

| Type                                       | Description                                                                                  |
|--------------------------------------------|----------------------------------------------------------------------------------------------|
| `StateStore`                               | A consensus state backend, with read (`StateStoreReadTransaction`) and write transactions    |
| `CommittedBlockProof`                      | A block header with the chain of quorum certificates that committed it                       |
| `VerifiedBlockTip`                         | The quorum-signed header fields of a block whose commit proof validated                      |
| `EpochCheckpoint`                          | A shard group's end-of-epoch checkpoint, which names the next epoch's validator set          |
| `TrustedStateRoot` / `DownProofAnchor`     | State roots taken from verified commit proofs, which substate and down proofs verify against |
| `verify_substate_value_proof_against_root` | Verifies a substate's value, or its absence, against a trusted state root                    |
| `verify_substate_down_proof_against_roots` | Verifies that a substate version went down, against the roots of two committed blocks        |

## Features

- **`ts`**: generates TypeScript type definitions via `ts-rs`.

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
