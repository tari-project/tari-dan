# Fixtures

## How to update the commit proof fixture

`commit_proof.json` is a JSON-serialized `SidechainBlockCommitProof` produced by the ootle repo.

## Encoding fixtures

`commit_proof.reserialized.json` and `commit_proof.borsh.hex` are the JSON and Borsh encodings of the proof in
`commit_proof.json`, as `tari_sidechain` 6.2.0 wrote them. `tests/golden_vectors.rs` checks that this crate still
writes the same bytes. Never regenerate them to make a failing test pass: a mismatch means the wire format changed.

`claim_burn_proof.json` is a `CompleteClaimBurnProof` as the 6.2.0 console wallet writes it. The Ootle wallet must
keep parsing claim files written by older wallets.
