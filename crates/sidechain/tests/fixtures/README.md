# Fixtures

## How to update the commit proof fixture

`commit_proof.json` is a JSON-serialized `SidechainBlockCommitProof` over a test header, committed by a 3-chain of
certificates that all four members of `tests/support/mod.rs::fixture_committee` sign. Each certificate carries the
signer bitmap `0f` and its signatures in ascending order of the signers' public keys.

The `the_fixture_is_the_proof_the_fixture_committee_signs` test checks that the file matches the proof the test
support builds. After changing that proof, regenerate the file:

```sh
cargo test -p tari_sidechain --test commit_proof -- --ignored regenerate_fixture
```

## Encoding fixtures

`tests/golden_vectors.rs` checks that this crate still writes the proof in `commit_proof.json` as that JSON, and as the
Borsh bytes in `commit_proof.borsh.hex`. Never regenerate either file to make a failing golden vector pass: a mismatch
means the wire format changed, which only a network reset allows.

`claim_burn_proof.json` is a `CompleteClaimBurnProof` as the 6.2.0 console wallet writes it. The Ootle wallet must
keep parsing claim files written by older wallets.
