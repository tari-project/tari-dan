# tari_ootle_light_client

[![Crates.io](https://img.shields.io/crates/v/tari_ootle_light_client.svg)](https://crates.io/crates/tari_ootle_light_client)
[![Documentation](https://docs.rs/tari_ootle_light_client/badge.svg)](https://docs.rs/tari_ootle_light_client)

A light client for Tari Ootle wallets. It follows the network's validator set from epoch to epoch and verifies what
an indexer serves against it, so a wallet does not have to trust the indexer for substates, receipts or downs.

Trust starts at an anchor: either the validator set the latest end-of-epoch checkpoint names (trust on first use), or
a pinned validator set hash. From there, each end-of-epoch checkpoint, signed by the trusted committee, hands trust
over to the next epoch's validator set. A committee of a trusted set then verifies commit proofs, and through them
substate, receipt and down proofs.

## Key types

| Type                         | Description                                                                                          |
|------------------------------|------------------------------------------------------------------------------------------------------|
| `TrustedValidatorSet`        | The latest trusted validator set, with the committee of the shard group the light client walks       |
| `VerifiedCommittee`          | A shard group's committee, proved to be in a trusted validator set; verifies commit and state proofs |
| `plan_walk` / `WalkPlan`     | Whether to walk forward to the latest checkpoint or re-anchor because trust is too far behind        |
| `verify_down_proof`          | Verifies that a substate version went down, with the committees of the proof's two anchor blocks     |
| `check_diff_matches_receipt` | Checks that a substate diff is the one a verified transaction receipt commits to                     |

## Example

```rust,ignore
use tari_ootle_light_client::{DEFAULT_MAX_WALK_EPOCHS, TrustedValidatorSet, WalkPlan, plan_walk};

// First use: trust the validator set that the latest checkpoint names.
let mut trusted = TrustedValidatorSet::anchor_on_first_use(network, walk_shard, &latest_checkpoint)?;

// Later: catch up through each end-of-epoch checkpoint since.
match plan_walk(&trusted, latest_checkpoint_epoch, DEFAULT_MAX_WALK_EPOCHS) {
    WalkPlan::UpToDate => {},
    WalkPlan::Walk { from_epoch, to_epoch } => {
        for epoch in from_epoch.as_u64()..=to_epoch.as_u64() {
            trusted = trusted.advance(&fetch_checkpoint(epoch)?)?;
        }
    },
    WalkPlan::Reanchor => {
        trusted = TrustedValidatorSet::anchor_on_first_use(network, walk_shard, &latest_checkpoint)?;
    },
}

// Verify a substate the indexer served, with the committee of the block its proof is anchored at. The value comes
// from the proof's committed encoding; use it in place of the substate served beside the proof.
let committee = trusted.verify_committee(&served.committee)?;
let (tip, value) = committee.verify_substate(&substate_id, version, &served.proof)?;
```

A failed `advance` leaves the trusted set as it was. Proofs anchored in an earlier epoch verify with a committee of
that epoch's set, so a caller that verifies them keeps each set it walks through.

## License

BSD-3-Clause. Copyright 2026 The Tari Project.
