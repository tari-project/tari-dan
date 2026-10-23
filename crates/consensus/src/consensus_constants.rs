//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::time::Duration;

use tari_engine_types::fees::ExhaustBurnRate;
use tari_ootle_common_types::{MAX_BLOCK_SHARD_OUTPUT_BYTES, MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES, NumPreshards};
use tari_ootle_storage::consensus_models::LivenessThresholds;
use tari_ootle_transaction::Network;

/// The byte cap every network uses. Keeping it uniform is what lets one gossip limit serve the whole
/// network.
///
/// Stated outright rather than derived from `ENGINE_LIMITS.max_template_binary_size_bytes`, even
/// though it has to admit a max-size template publish. Deriving it made this — and the gossip frame
/// limit built on it (`tari_ootle_p2p::max_gossip_message_size`) — move silently whenever the
/// template limit moved, and the two are not the same decision: the template limit is about what one
/// instruction costs to execute, this is about what the whole network agrees to relay, and a node
/// that disagrees with its peers about it rejects their transactions as codec errors rather than as
/// invalid ones.
///
/// `the_transaction_size_cap_still_admits_a_max_size_publish` fails if the two drift apart, so a
/// change to either is reviewed against the other rather than inherited.
const MAX_TRANSACTION_SIZE_BYTES: usize = 1_310_720; // 1 MiB + 256 KiB

/// A max-size substate with half a MiB to spare, so a transaction reading one is never aborted.
const MAX_BLOCK_PLEDGE_BYTES: usize = 1536 * 1024;

/// Leaves 1.5 MiB of the 4 MiB transport frame for the local block the foreign proposals travel with. Nothing bounds
/// a block's encoded size directly: the 1.5 MiB is an estimate of what `max_block_weight` and
/// `max_commands_in_block` admit.
const MAX_BLOCK_FOREIGN_PROPOSAL_BYTES: usize = 2560 * 1024;

/// Half a MiB above the proposal budget, which absorbs the differences between copies of the same foreign proposal.
const MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES: usize = 3 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ConsensusConstants {
    /// Number of base layer confirmations required before an L1 block is considered unable to re-org.
    pub base_layer_confirmations: u64,
    /// The target size of the committee per shard group.
    pub committee_size_per_shard_group: u32,
    /// The number of preshards to break up the shard space.
    pub num_preshards: NumPreshards,
    /// The maximum block time. The pacemaker will trigger a new view if a block is not received within this time +
    /// delta.
    pub pacemaker_block_time: Duration,
    /// The number of missed proposals that suspends a validator. A suspended validator's slot in the
    /// leader rotation is remapped to the next validator that is not suspended, so the network does
    /// not pay a pacemaker timeout each time a validator that has stopped proposing comes up.
    /// CONSENSUS RULE: must be uniform network-wide, otherwise nodes disagree on who may propose.
    pub missed_proposal_suspend_threshold: u64,
    /// The number of votes a suspended validator must land in committed blocks to earn its first
    /// probation slot - one slot in which it is the leader again and can prove it is back. The
    /// requirement doubles with each probation slot it then misses, up to
    /// `probation_max_backoff_exp`, so a validator that votes but never proposes costs the network a
    /// geometrically decreasing number of slots.
    /// CONSENSUS RULE: must be uniform network-wide, otherwise nodes disagree on who may propose.
    pub probation_base_votes: u64,
    /// The number of committed blocks after which a suspended validator gets a probation slot even
    /// if none of its votes reached a certificate. This is what guarantees that a validator gets a
    /// slot back: a vote counts towards the slot only once it is in the certificate of a committed
    /// block and a leader stops collecting at a quorum, so a validator that is merely slower than
    /// its committee can have every vote of its arrive too late to count. Doubles with each missed
    /// probation slot like `probation_base_votes`.
    /// CONSENSUS RULE: must be uniform network-wide, otherwise nodes disagree on who may propose.
    pub probation_base_blocks: u64,
    /// The cap on the exponent in `probation_base_votes × 2^probation_failures` and
    /// `probation_base_blocks × 2^probation_failures`.
    /// CONSENSUS RULE: must be uniform network-wide, otherwise nodes disagree on who may propose.
    pub probation_max_backoff_exp: u32,
    /// The maximum total weight of commands a leader will pack into a single block. This is a budget
    /// of transaction weight (see `Transaction::calculate_transaction_weight`) rather than a flat
    /// command count, so heavy transactions consume more of a block than light ones. This is a local
    /// proposing heuristic only — it is not validated when receiving/voting on a block, so it carries
    /// no fork risk and nodes may run different values.
    pub max_block_weight: u64,
    /// A hard upper bound on the number of commands in a block, independent of weight. Bounds the
    /// on-the-wire/`BTreeSet` overhead so a flood of near-zero-weight commands cannot bloat a block.
    /// Like `max_block_weight`, this is a propose-time heuristic and is not validated on receive.
    pub max_commands_in_block: usize,
    /// The maximum total transaction execution weight a block may contain to be considered valid. Unlike
    /// `max_block_weight` (a local proposing heuristic) this IS enforced when receiving/voting on a block:
    /// a block whose transaction execution weight exceeds this — and that contains more than one
    /// transaction command — is rejected (no-vote). It bounds how long a replica can be made to spend
    /// executing a block, preventing a misbehaving leader from pushing replicas past the block time.
    /// Set above `max_block_weight` so honest proposals are never rejected. CONSENSUS RULE: must be
    /// uniform network-wide, otherwise nodes diverge on block validity.
    pub max_block_validation_weight: u64,
    /// The maximum weight (see `Transaction::calculate_transaction_weight`) a single transaction may
    /// have to be admitted. Unlike the block-level budgets this bounds an *individual* transaction,
    /// stopping one transaction from carrying a disproportionate amount of size/IO/execution work
    /// (e.g. multi-MiB inline arguments or a flood of cheap instructions) into the network before it
    /// is gossiped, stored and executed. Must sit above the heaviest legitimate transaction — a
    /// max-size template publish (`limits::ENGINE_LIMITS.max_template_binary_size_bytes`) — so honest
    /// transactions are never rejected. Enforced at RPC submit / mempool admission, not in consensus.
    pub max_transaction_weight: u64,
    /// The maximum encoded size (`Transaction::encoded_size`) a single transaction may have to be
    /// admitted.
    ///
    /// `max_transaction_weight` bounds work, not bytes: blob payloads are charged at a divisor, so a
    /// transaction can carry several times this many bytes and still weigh under the cap. This bounds
    /// the bytes directly, which is what the gossip layer has to admit — a node whose gossip limit is
    /// below what ingress accepts refuses valid transactions as a codec frame error, so the two must
    /// be set together (see `tari_ootle_p2p::max_gossip_message_size`).
    ///
    /// Must sit above a max-size template publish
    /// (`limits::ENGINE_LIMITS.max_template_binary_size_bytes` plus its transaction envelope) so
    /// honest publishes are never rejected. Enforced at RPC submit / mempool admission, not in
    /// consensus, but every node must agree on it: a node admitting more than its peers gossips
    /// transactions they will not relay.
    pub max_transaction_size_bytes: usize,
    /// The maximum total execution points a leader will pack into a single block, summed from each transaction's
    /// actual cost (`ExecuteResult::total_execution_points`): WASM metering plus native crypto verification.
    /// Transaction weight is size/IO-based and blind to execution cost, so a low-weight compute-heavy
    /// transaction evades the weight budget — this bounds the compute a block adds on top of the weight-bounded
    /// work. Native verification is counted here rather than approximated by transaction weight, so a block full
    /// of stealth or confidential statements is governed by the same budget as one full of WASM. Like
    /// `max_block_weight`, this is a local proposing heuristic only and carries no fork risk. A transaction's
    /// points are only known after it executes, so a block may overshoot this budget by up to
    /// `MAX_WASM_POINTS_PER_TRANSACTION + MAX_NATIVE_POINTS_PER_TRANSACTION`;
    /// `max_block_validation_execution_points` must allow for this.
    pub max_block_execution_points: u64,
    /// The maximum total execution points a block may contain to be considered valid. Like
    /// `max_block_validation_weight` this IS enforced when receiving/voting on a block: a replica keeps a
    /// running points total while executing the block's commands and stops at the first command that pushes the
    /// total over this limit (no-vote), bounding the CPU a misbehaving leader can extract from replicas. Both
    /// halves are deterministic — metering is, and the native price is a pure function of the declared statement
    /// — so every replica stops at the same command and votes identically. Must be at least
    /// `max_block_execution_points + MAX_WASM_POINTS_PER_TRANSACTION + MAX_NATIVE_POINTS_PER_TRANSACTION` so
    /// honest proposals are never rejected.
    /// CONSENSUS RULE: must be uniform network-wide, otherwise nodes diverge on block validity.
    pub max_block_validation_execution_points: u64,
    /// The most encoded substate bytes a leader puts up in any one shard in a block. A transaction whose
    /// diff would take a shard past it is deferred to a later block. The proposing half of
    /// `max_block_validation_shard_output_bytes`.
    pub max_block_shard_output_bytes: usize,
    /// The most encoded substate bytes a block may put up in any one shard to be voted for. A block's
    /// writes to a shard form one state version, which state sync transfers and commits whole, so this is
    /// what bounds the size of a version. CONSENSUS RULE: must be uniform network-wide, otherwise nodes
    /// diverge on block validity.
    pub max_block_validation_shard_output_bytes: usize,
    /// The most encoded input substate bytes a block may pledge to foreign shard groups, counting each
    /// pledged substate once however many of the block's commands pledge it. Every foreign shard group
    /// receives a subset of these as the block's pledge, inline in a single foreign proposal, so this is
    /// what keeps each foreign proposal small enough to deliver. It also caps what one transaction may
    /// pledge: a transaction whose local inputs alone exceed it is aborted, because no block could carry
    /// its pledge. A leader defers a transaction that would take the block past it. Must admit a
    /// max-size substate (`limits::ENGINE_LIMITS.max_substate_size`) so a transaction that reads one is
    /// never aborted. CONSENSUS RULE: must be uniform network-wide, otherwise nodes diverge on block
    /// validity and on which transactions abort.
    pub max_block_pledge_bytes: usize,
    /// The most encoded bytes of foreign proposals a leader packs into a block that carries more than
    /// one. A local proposal travels with every foreign proposal it includes, in one message that must
    /// fit the transport's frame limit (4 MiB), so this must leave room for the block itself. That room
    /// is an estimate rather than an enforced bound, because a block's encoded size is limited only
    /// through its weight and command count. A single foreign proposal is always admitted: its pledge is
    /// bounded by `max_block_pledge_bytes`, and holding it back would leave it unproposable. The
    /// proposing half of `max_block_validation_foreign_proposal_bytes`.
    pub max_block_foreign_proposal_bytes: usize,
    /// The most encoded bytes of foreign proposals a block that carries more than one may carry to be
    /// voted for. Each node measures its own stored copy of each foreign proposal, and copies of one
    /// proposal can differ in their commit proofs (the certificates each sender observed), so this sits
    /// above `max_block_foreign_proposal_bytes` by enough that a validator's copies never take an
    /// honest block over it. CONSENSUS RULE: must be uniform network-wide, otherwise nodes diverge on
    /// block validity.
    pub max_block_validation_foreign_proposal_bytes: usize,
    /// The share of collected fees that is burned rather than paid to leaders, in basis points, for a
    /// context that has no epoch to resolve against: a dry run, a fee estimate, a test harness. The
    /// user's price is the fee table alone; this only splits what was collected.
    ///
    /// Consensus does not read it. The rate an epoch actually settles at is resolved once at the epoch
    /// boundary and carried in every block header of that epoch — see `ExhaustBurnRateSchedule` and
    /// `LockedEpoch::exhaust_burn_rate` — so a node that read this field would be settling against a
    /// constant the network may have moved away from.
    pub exhaust_burn_rate: ExhaustBurnRate,
    /// The furthest ahead of the current epoch a transaction's `max_epoch` may be set. Every
    /// transaction declares a mandatory `max_epoch`, so this caps how long any transaction can
    /// remain sequenceable: a wallet can declare a transaction permanently dead once this many
    /// epochs have passed, and an aborted attempt — which consensus deliberately allows to be
    /// re-sequenced — cannot be retried beyond its window. This is a ceiling, not a default —
    /// wallets stamp a much shorter window for ordinary traffic and only long-lived flows
    /// (offline or multi-party signing) approach it. Enforced at mempool admission and in
    /// consensus sequencing. CONSENSUS RULE: must be uniform network-wide, otherwise nodes
    /// diverge on which transactions may be sequenced.
    pub max_transaction_validity_epochs: u64,
}

impl ConsensusConstants {
    /// The committee size [`Self::DEVNET`] is built at. A devnet that wants another size sets it in a
    /// consensus constants file, which every node on that network is handed.
    pub const DEFAULT_DEVNET_COMMITTEE_SIZE: u32 = 7;
    /// Devnet at the default committee size.
    pub const DEVNET: Self = Self::devnet(Self::DEFAULT_DEVNET_COMMITTEE_SIZE);
    pub const ESMERALDA: Self = Self {
        base_layer_confirmations: 100,
        committee_size_per_shard_group: 40,
        num_preshards: NumPreshards::current(),
        pacemaker_block_time: Duration::from_secs(10),
        missed_proposal_suspend_threshold: 5,
        probation_base_votes: 5,
        probation_base_blocks: 100,
        probation_max_backoff_exp: 6,
        // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
        // transactions executed in ~11.5s. `INVOCATION_FLOOR` denominates the weight unit: every
        // instruction that instantiates a template weighs at least the floor, which puts a plain
        // transfer (three invocations, two inputs, one signer) at ~125 and the stress transaction
        // at ~150, giving ~6.5k weight/s. A 24000 budget is ~160 of those commands and projects to
        // ~3.7s of propose-time execution, comfortably within the 10s block time and under the 5s
        // execution circuit breaker, while heavier transactions naturally consume more of the
        // budget. The breaker in on_propose is the backstop for outliers the static weight
        // under-estimates. `the_block_budget_admits_the_calibrated_command_count` pins the command
        // count so a change to the floor cannot silently shrink the block. NOTE: propose-time
        // execution is sequential, so more cores does not raise this proportionally — calibrate to
        // single-core throughput.
        max_block_weight: 24_000,
        max_commands_in_block: 1000,
        // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
        // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
        // within the 10s block time. Rejects the ~500-command overload that broke things.
        max_block_validation_weight: 36_000,
        // Admits the heaviest legitimate transaction — a 1 MiB template publish is ~350k weight
        // (binary bytes / 3) — with ~3x headroom, while bounding any single transaction's
        // size/execution cost at ingress. A mempool admission bound, not a consensus rule.
        max_transaction_weight: 1_000_000,
        max_transaction_size_bytes: MAX_TRANSACTION_SIZE_BYTES,
        // ~18 max-compute transactions (`MAX_WASM_POINTS_PER_TRANSACTION` each) — ~536ms of serial
        // execution at the calibrated ~8.4M points/ms, ~5% of the block time, leaving the rest for
        // consensus, storage and slower validator hardware. The transaction count is a floor the
        // per-transaction ceiling is sized against, asserted by
        // `a_block_admits_enough_max_compute_transactions`.
        max_block_execution_points: 4_500_000_000,
        // Proposal budget + the largest single-transaction overshoot the per-transaction ceilings allow
        // (`MAX_WASM_POINTS_PER_TRANSACTION` + `MAX_NATIVE_POINTS_PER_TRANSACTION`) + margin, so honest
        // proposals are never rejected.
        max_block_validation_execution_points: 7_250_000_000,
        max_block_shard_output_bytes: MAX_BLOCK_SHARD_OUTPUT_BYTES,
        max_block_validation_shard_output_bytes: MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES,
        max_block_pledge_bytes: MAX_BLOCK_PLEDGE_BYTES,
        max_block_foreign_proposal_bytes: MAX_BLOCK_FOREIGN_PROPOSAL_BYTES,
        max_block_validation_foreign_proposal_bytes: MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES,
        exhaust_burn_rate: ExhaustBurnRate::new(500), // 5%
        max_transaction_validity_epochs: 2160,
    };
    pub const MAINNET: Self = Self {
        // Minotari's `coinbase_min_maturity` (720) plus 60 blocks (~2 hours) of margin. Must stay a
        // multiple of the L1 `vn_epoch_length` — 60 today, 10 once the shorter-epoch fork lands — so
        // the lag ends on an epoch boundary, leaving the wallet's "strictly past the mined-in epoch"
        // gate as the only rounding a claimant sees. The depth is deliberately generous because the
        // bound is one-way: a burn claim proved against a header that a deeper reorg later orphans has
        // already credited L2 state that consensus cannot roll back, whereas an over-deep lag only
        // costs the claimant time. At mainnet's 2 minute block time a burn becomes claimable
        // ~26 hours after it is mined.
        base_layer_confirmations: 780,
        committee_size_per_shard_group: 40,
        num_preshards: NumPreshards::current(),
        pacemaker_block_time: Duration::from_secs(10),
        missed_proposal_suspend_threshold: 5,
        probation_base_votes: 5,
        probation_base_blocks: 100,
        probation_max_backoff_exp: 6,
        // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
        // transactions executed in ~11.5s. `INVOCATION_FLOOR` denominates the weight unit: every
        // instruction that instantiates a template weighs at least the floor, which puts a plain
        // transfer (three invocations, two inputs, one signer) at ~125 and the stress transaction
        // at ~150, giving ~6.5k weight/s. A 24000 budget is ~160 of those commands and projects to
        // ~3.7s of propose-time execution, comfortably within the 10s block time and under the 5s
        // execution circuit breaker, while heavier transactions naturally consume more of the
        // budget. The breaker in on_propose is the backstop for outliers the static weight
        // under-estimates. `the_block_budget_admits_the_calibrated_command_count` pins the command
        // count so a change to the floor cannot silently shrink the block. NOTE: propose-time
        // execution is sequential, so more cores does not raise this proportionally — calibrate to
        // single-core throughput.
        max_block_weight: 24_000,
        max_commands_in_block: 1000,
        // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
        // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
        // within the 10s block time. Rejects the ~500-command overload that broke things.
        max_block_validation_weight: 36_000,
        // Admits the heaviest legitimate transaction — a 1 MiB template publish is ~350k weight
        // (binary bytes / 3) — with ~3x headroom, while bounding any single transaction's
        // size/execution cost at ingress. A mempool admission bound, not a consensus rule.
        max_transaction_weight: 1_000_000,
        max_transaction_size_bytes: MAX_TRANSACTION_SIZE_BYTES,
        // ~18 max-compute transactions (`MAX_WASM_POINTS_PER_TRANSACTION` each) — ~536ms of serial
        // execution at the calibrated ~8.4M points/ms, ~5% of the block time, leaving the rest for
        // consensus, storage and slower validator hardware. The transaction count is a floor the
        // per-transaction ceiling is sized against, asserted by
        // `a_block_admits_enough_max_compute_transactions`.
        max_block_execution_points: 4_500_000_000,
        // Proposal budget + the largest single-transaction overshoot the per-transaction ceilings allow
        // (`MAX_WASM_POINTS_PER_TRANSACTION` + `MAX_NATIVE_POINTS_PER_TRANSACTION`) + margin, so honest
        // proposals are never rejected.
        max_block_validation_execution_points: 7_250_000_000,
        max_block_shard_output_bytes: MAX_BLOCK_SHARD_OUTPUT_BYTES,
        max_block_validation_shard_output_bytes: MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES,
        max_block_pledge_bytes: MAX_BLOCK_PLEDGE_BYTES,
        max_block_foreign_proposal_bytes: MAX_BLOCK_FOREIGN_PROPOSAL_BYTES,
        max_block_validation_foreign_proposal_bytes: MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES,
        exhaust_burn_rate: ExhaustBurnRate::new(500), // 5%
        max_transaction_validity_epochs: 2160,
    };
    pub const TESTNET: Self = Self {
        base_layer_confirmations: 100,
        committee_size_per_shard_group: 40,
        num_preshards: NumPreshards::current(),
        pacemaker_block_time: Duration::from_secs(10),
        missed_proposal_suspend_threshold: 5,
        probation_base_votes: 5,
        probation_base_blocks: 100,
        probation_max_backoff_exp: 6,
        // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
        // transactions executed in ~11.5s. `INVOCATION_FLOOR` denominates the weight unit: every
        // instruction that instantiates a template weighs at least the floor, which puts a plain
        // transfer (three invocations, two inputs, one signer) at ~125 and the stress transaction
        // at ~150, giving ~6.5k weight/s. A 24000 budget is ~160 of those commands and projects to
        // ~3.7s of propose-time execution, comfortably within the 10s block time and under the 5s
        // execution circuit breaker, while heavier transactions naturally consume more of the
        // budget. The breaker in on_propose is the backstop for outliers the static weight
        // under-estimates. `the_block_budget_admits_the_calibrated_command_count` pins the command
        // count so a change to the floor cannot silently shrink the block. NOTE: propose-time
        // execution is sequential, so more cores does not raise this proportionally — calibrate to
        // single-core throughput.
        max_block_weight: 24_000,
        max_commands_in_block: 1000,
        // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
        // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
        // within the 10s block time. Rejects the ~500-command overload that broke things.
        max_block_validation_weight: 36_000,
        // Admits the heaviest legitimate transaction — a 1 MiB template publish is ~350k weight
        // (binary bytes / 3) — with ~3x headroom, while bounding any single transaction's
        // size/execution cost at ingress. A mempool admission bound, not a consensus rule.
        max_transaction_weight: 1_000_000,
        max_transaction_size_bytes: MAX_TRANSACTION_SIZE_BYTES,
        // ~18 max-compute transactions (`MAX_WASM_POINTS_PER_TRANSACTION` each) — ~536ms of serial
        // execution at the calibrated ~8.4M points/ms, ~5% of the block time, leaving the rest for
        // consensus, storage and slower validator hardware. The transaction count is a floor the
        // per-transaction ceiling is sized against, asserted by
        // `a_block_admits_enough_max_compute_transactions`.
        max_block_execution_points: 4_500_000_000,
        // Proposal budget + the largest single-transaction overshoot the per-transaction ceilings allow
        // (`MAX_WASM_POINTS_PER_TRANSACTION` + `MAX_NATIVE_POINTS_PER_TRANSACTION`) + margin, so honest
        // proposals are never rejected.
        max_block_validation_execution_points: 7_250_000_000,
        max_block_shard_output_bytes: MAX_BLOCK_SHARD_OUTPUT_BYTES,
        max_block_validation_shard_output_bytes: MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES,
        max_block_pledge_bytes: MAX_BLOCK_PLEDGE_BYTES,
        max_block_foreign_proposal_bytes: MAX_BLOCK_FOREIGN_PROPOSAL_BYTES,
        max_block_validation_foreign_proposal_bytes: MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES,
        exhaust_burn_rate: ExhaustBurnRate::new(500), // 5%
        max_transaction_validity_epochs: 2160,
    };

    pub const fn mainnet() -> Self {
        Self::MAINNET
    }

    pub const fn esmeralda() -> Self {
        Self::ESMERALDA
    }

    pub const fn testnet() -> Self {
        Self::TESTNET
    }

    pub const fn devnet(committee_size: u32) -> Self {
        Self {
            base_layer_confirmations: 3,
            committee_size_per_shard_group: committee_size,
            num_preshards: NumPreshards::current(),
            pacemaker_block_time: Duration::from_secs(10),
            missed_proposal_suspend_threshold: 5,
            probation_base_votes: 5,
            probation_base_blocks: 100,
            probation_max_backoff_exp: 6,
            // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
            // transactions executed in ~11.5s. `INVOCATION_FLOOR` denominates the weight unit: every
            // instruction that instantiates a template weighs at least the floor, which puts a plain
            // transfer (three invocations, two inputs, one signer) at ~125 and the stress transaction
            // at ~150, giving ~6.5k weight/s. A 24000 budget is ~160 of those commands and projects to
            // ~3.7s of propose-time execution, comfortably within the 10s block time and under the 5s
            // execution circuit breaker, while heavier transactions naturally consume more of the
            // budget. The breaker in on_propose is the backstop for outliers the static weight
            // under-estimates. `the_block_budget_admits_the_calibrated_command_count` pins the command
            // count so a change to the floor cannot silently shrink the block. NOTE: propose-time
            // execution is sequential, so more cores does not raise this proportionally — calibrate to
            // single-core throughput.
            max_block_weight: 24_000,
            max_commands_in_block: 1000,
            // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
            // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
            // within the 10s block time. Rejects the ~500-command overload that broke things.
            max_block_validation_weight: 36_000,
            // Admits the heaviest legitimate transaction — a 1 MiB template publish is ~350k weight
            // (binary bytes / 3) — with ~3x headroom, while bounding any single transaction's
            // size/execution cost at ingress. A mempool admission bound, not a consensus rule.
            max_transaction_weight: 1_000_000,
            max_transaction_size_bytes: MAX_TRANSACTION_SIZE_BYTES,
            // ~18 max-compute transactions (`MAX_WASM_POINTS_PER_TRANSACTION` each) — ~536ms of serial
            // execution at the calibrated ~8.4M points/ms, ~5% of the block time, leaving the rest for
            // consensus, storage and slower validator hardware. The transaction count is a floor the
            // per-transaction ceiling is sized against, asserted by
            // `a_block_admits_enough_max_compute_transactions`.
            max_block_execution_points: 4_500_000_000,
            // Proposal budget + the largest single-transaction overshoot the per-transaction ceilings allow
            // (`MAX_WASM_POINTS_PER_TRANSACTION` + `MAX_NATIVE_POINTS_PER_TRANSACTION`) + margin, so honest
            // proposals are never rejected.
            max_block_validation_execution_points: 7_250_000_000,
            max_block_shard_output_bytes: MAX_BLOCK_SHARD_OUTPUT_BYTES,
            max_block_validation_shard_output_bytes: MAX_BLOCK_VALIDATION_SHARD_OUTPUT_BYTES,
            max_block_pledge_bytes: MAX_BLOCK_PLEDGE_BYTES,
            max_block_foreign_proposal_bytes: MAX_BLOCK_FOREIGN_PROPOSAL_BYTES,
            max_block_validation_foreign_proposal_bytes: MAX_BLOCK_VALIDATION_FOREIGN_PROPOSAL_BYTES,
            exhaust_burn_rate: ExhaustBurnRate::new(500), // 5%
            max_transaction_validity_epochs: 2160,
        }
    }

    /// The thresholds that decide a validator's liveness state, and with it whether leader selection
    /// skips its slot.
    pub fn liveness_thresholds(&self) -> LivenessThresholds {
        LivenessThresholds {
            suspend_after_missed: self.missed_proposal_suspend_threshold,
            probation_base_votes: self.probation_base_votes,
            probation_base_blocks: self.probation_base_blocks,
            probation_max_backoff_exp: self.probation_max_backoff_exp,
        }
    }
}

/// Forces [`ConsensusConstants::DEVNET`] to be evaluated, and with it `devnet`'s body. The other
/// networks are const items their constructors return, so they are evaluated wherever they are
/// built; `devnet` takes an argument and builds inline. The runtime reference in `From<Network>`
/// below is not enough on its own: rustc was observed not to evaluate `DEVNET` for it, the
/// initializer being a `const fn` call. This item does not depend on that — a const context
/// evaluates what it names. It pins `devnet` at the default committee size, which covers the whole
/// body: nothing in it varies with the argument beyond the field it sets.
const _: ConsensusConstants = ConsensusConstants::DEVNET;

impl From<Network> for ConsensusConstants {
    fn from(network: Network) -> Self {
        match network {
            Network::MainNet => Self::mainnet(),
            Network::LocalNet => Self::DEVNET,
            Network::Esmeralda => Self::esmeralda(),
            Network::StageNet | Network::NextNet | Network::Igor => Self::testnet(),
        }
    }
}

#[cfg(test)]
mod tests {
    use tari_ootle_common_types::Epoch;

    /// Room above a template binary for the rest of the transaction carrying it: its other
    /// instructions, inputs, signatures and CBOR framing.
    ///
    /// Deliberately loose against those — a real max-size publish encodes to a couple of hundred
    /// bytes over its binary, and a single instruction is capped at `ENGINE_LIMITS.max_call_size` —
    /// because a legitimate transaction refused at ingress is a worse failure than the bytes a
    /// larger allowance costs. `max_transaction_size_admits_max_template_publish` holds it to that.
    const TRANSACTION_ENVELOPE_ALLOWANCE: usize = 256 * 1024;

    use tari_common_types::types::PrivateKey;
    use tari_engine_types::{
        limits::{
            ENGINE_LIMITS,
            MAX_NATIVE_POINTS_PER_TRANSACTION,
            MAX_WASM_POINTS_PER_TRANSACTION,
            MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK,
        },
        substate::SubstateId,
    };
    use tari_ootle_common_types::InputDeclaration;
    use tari_ootle_transaction::{
        INVOCATION_FLOOR,
        MAX_SIGNATURES_PER_TRANSACTION,
        MIN_INVOCATION_ENCODED_BYTES,
        Sr25519SignatureBytes,
        Transaction,
        TransactionSignature,
        args,
    };
    use tari_template_lib_types::{
        Amount,
        ComponentAddress,
        ObjectKey,
        constants::TARI_TOKEN,
        crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes},
        stealth::{StealthInput, StealthInputsStatement, StealthOutputsStatement, StealthTransferStatement},
    };

    use super::*;

    /// A consensus rule must be identical on every network, otherwise a transaction admitted on one
    /// is refused on another and nodes diverge on which transactions may be sequenced.
    #[test]
    fn the_transaction_validity_ceiling_is_uniform_across_networks() {
        let expected = ConsensusConstants::mainnet().max_transaction_validity_epochs;
        for constants in [
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert_eq!(constants.max_transaction_validity_epochs, expected);
        }
        // A zero ceiling would admit only transactions expiring in the current epoch, leaving no
        // room to submit one at all.
        assert!(expected > 0);
    }

    #[test]
    fn validation_budgets_always_admit_honest_proposals() {
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert!(constants.max_block_validation_weight >= constants.max_block_weight);
            // A leader only learns a transaction's points after executing it, so an honest block may exceed
            // the propose budget by up to one transaction's full budget — both halves of it, since the
            // execution budget covers native verification as well as WASM.
            assert!(
                constants.max_block_validation_execution_points >=
                    constants.max_block_execution_points +
                        MAX_WASM_POINTS_PER_TRANSACTION +
                        MAX_NATIVE_POINTS_PER_TRANSACTION
            );
            // A leader checks a transaction's diff against the budget before putting it, so it never overshoots.
            assert!(constants.max_block_validation_shard_output_bytes >= constants.max_block_shard_output_bytes);
            // A validator measures its own copies of the foreign proposals, whose commit proofs may be larger than
            // the leader's.
            assert!(
                constants.max_block_validation_foreign_proposal_bytes >=
                    constants.max_block_foreign_proposal_bytes + 256 * 1024
            );
        }
    }

    /// A leader defers a transaction whose diff would take a shard past the block budget. One whose diff alone
    /// is over it could never be proposed, so the engine's per-transaction output cap must fit in an empty block.
    #[test]
    fn a_max_output_transaction_fits_in_a_block() {
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert!(
                constants.max_block_shard_output_bytes >=
                    tari_engine_types::limits::ENGINE_LIMITS.max_transaction_output_bytes
            );
        }
    }

    /// A transaction whose pledge alone is over the block's pledge budget is aborted, so the budget must admit a
    /// max-size substate with room for its id. A local block carries several foreign proposals in one message, so
    /// their budget must leave room for the block within the 4 MiB transport frame. A block's encoded size is not
    /// bounded directly, so the room is an estimate of what the block weight and command caps admit.
    #[test]
    fn pledges_and_foreign_proposals_fit_the_frame() {
        const FRAME_BYTES: usize = 4 * 1024 * 1024;
        const ESTIMATED_BLOCK_ROOM_BYTES: usize = 1024 * 1024;
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert!(
                constants.max_block_pledge_bytes >=
                    tari_engine_types::limits::ENGINE_LIMITS.max_substate_size + 64 * 1024
            );
            assert!(constants.max_block_foreign_proposal_bytes >= constants.max_block_pledge_bytes);
            assert!(constants.max_block_foreign_proposal_bytes + ESTIMATED_BLOCK_ROOM_BYTES <= FRAME_BYTES);
        }
    }

    /// The per-transaction compute ceiling is a fraction of the block's, not an independent knob:
    /// raising it lets one transaction claim a larger share of a block, and past some point a leader
    /// packing max-compute transactions starves the block of everything else. This fixes how far
    /// that can go, so the two constants cannot drift apart silently.
    #[test]
    fn a_block_admits_enough_max_compute_transactions() {
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            let admitted = constants.max_block_execution_points / MAX_WASM_POINTS_PER_TRANSACTION;
            assert!(
                admitted >= MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK,
                "a block admits only {admitted} max-compute transactions, below the floor of \
                 {MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK}: either the per-transaction ceiling has outgrown the block \
                 budget, or the floor needs revisiting",
            );
        }
    }

    /// A transaction at [`MAX_SIGNATURES_PER_TRANSACTION`] must still fit the block a leader packs,
    /// or the cap admits transactions nothing can ever sequence.
    ///
    /// The shape that reaches the cap is a key-path stealth spend, which pays weight three times per
    /// stealth UTXO: once for the declared input, once for the signature authorizing it, and once
    /// for the statement input. So weight, not the signature cap, is what bounds such a
    /// transaction's share of a block — and the headroom between the two is a real quantity, built
    /// and measured here rather than reasoned about, since every term comes from a factor that can
    /// move independently.
    #[test]
    fn the_block_budget_admits_a_transaction_at_the_signature_cap() {
        let count = MAX_SIGNATURES_PER_TRANSACTION;

        // One stealth UTXO per signature: an input substate to lock, a statement input to aggregate,
        // and the one-time key's authorization.
        let inputs = (0..count).map(|i| {
            let mut key = [0u8; 32];
            key[..8].copy_from_slice(&(i as u64).to_le_bytes());
            InputDeclaration::write(SubstateId::Component(ComponentAddress::from_array(key)))
        });
        let statement = StealthTransferStatement {
            inputs_statement: StealthInputsStatement::new(
                (0..count)
                    .map(|i| {
                        let mut commitment = [0u8; 32];
                        commitment[..8].copy_from_slice(&(i as u64).to_le_bytes());
                        StealthInput::new(PedersenCommitmentBytes::from_bytes(&commitment).unwrap())
                    })
                    .collect(),
                Amount::zero(),
            ),
            outputs_statement: StealthOutputsStatement::new_revealed_only(
                Amount::zero(),
                RistrettoPublicKeyBytes::zero(),
            ),
            balance_proof: None,
            covenant_claims: vec![],
        };

        let weight = Transaction::builder_localnet(Epoch(1))
            .with_inputs(inputs)
            .stealth_transfer(TARI_TOKEN, statement)
            .with_signatures(
                (0..count)
                    .map(|_| TransactionSignature::new(RistrettoPublicKeyBytes::zero(), Sr25519SignatureBytes::zero()))
                    .collect(),
            )
            .seal(&PrivateKey::from(1u64))
            .calculate_transaction_weight()
            .as_u64();

        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert!(
                weight <= constants.max_block_weight,
                "a spend at the {count}-signature cap weighs {weight}, above the {} a leader packs — it could never \
                 be sequenced, so either the cap or the block budget has to move",
                constants.max_block_weight,
            );
            assert!(
                weight <= constants.max_transaction_weight,
                "a spend at the {count}-signature cap weighs {weight}, above the {} ingress admits",
                constants.max_transaction_weight,
            );
        }
    }

    /// The byte cap has the same obligation as the weight cap: a maximum-size template publish must
    /// still be admitted, envelope included.
    ///
    /// Built and measured rather than reasoned about, because `TRANSACTION_ENVELOPE_ALLOWANCE` is an
    /// estimate of encoding overhead and this is the assertion that keeps it honest — if the template
    /// limit or the transaction encoding moves, this fails rather than the network quietly refusing
    /// publishes.
    #[test]
    fn max_transaction_size_admits_max_template_publish() {
        let binary = vec![0u8; ENGINE_LIMITS.max_template_binary_size_bytes];
        let transaction = Transaction::builder_localnet(Epoch(1))
            .publish_template(binary)
            .build_and_seal(&PrivateKey::from(1u64));
        let size = transaction.encoded_size();

        assert!(
            size > ENGINE_LIMITS.max_template_binary_size_bytes,
            "a transaction carrying the binary must encode to at least the binary's size"
        );
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert!(
                constants.max_transaction_size_bytes >= size,
                "max_transaction_size_bytes ({}) must admit a max-size template publish ({size} bytes)",
                constants.max_transaction_size_bytes,
            );
        }
    }

    /// The byte cap must be the binding limit, not the weight cap: a transaction under the weight cap
    /// but over the byte cap is the case this exists to reject, and one over the weight cap but under
    /// the byte cap would mean the byte cap never fires.
    #[test]
    fn the_byte_cap_binds_before_the_weight_cap() {
        // Blob payloads are the cheapest bytes a transaction can carry — `calc_blobs_weight` charges
        // them at a divisor of 3 — so this is the most bytes the weight cap alone would admit.
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            let bytes_the_weight_cap_admits = constants.max_transaction_weight as usize * 3;
            assert!(
                constants.max_transaction_size_bytes < bytes_the_weight_cap_admits,
                "max_transaction_size_bytes ({}) is above the {} bytes max_transaction_weight already admits, so it \
                 would never reject anything",
                constants.max_transaction_size_bytes,
                bytes_the_weight_cap_admits,
            );
        }
    }

    /// `MAX_TRANSACTION_SIZE_BYTES` is stated rather than derived so that the gossip frame limit
    /// does not move on its own, which means nothing else forces it to keep admitting a max-size
    /// template publish. This is what does.
    ///
    /// A failure here is not automatically a bug: it means the template limit moved and somebody
    /// has to decide, deliberately, whether the network-wide byte cap and the gossip frame limit
    /// move with it.
    #[test]
    fn the_transaction_size_cap_still_admits_a_max_size_publish() {
        let derived = ENGINE_LIMITS.max_template_binary_size_bytes + TRANSACTION_ENVELOPE_ALLOWANCE;
        assert_eq!(
            MAX_TRANSACTION_SIZE_BYTES, derived,
            "the transaction byte cap ({MAX_TRANSACTION_SIZE_BYTES}) has drifted from the template binary limit plus \
             its envelope ({derived}). Changing either is a network-wide decision — it moves the gossip frame limit \
             with it — so update this constant deliberately rather than to make this pass."
        );
    }

    /// The byte cap is what bounds how many instructions a transaction can carry, and
    /// `INVOCATION_FLOOR` is what turns that into weight. If the floor is too low for the byte cap,
    /// a transaction packed with minimal template invocations passes the weight cap while still
    /// making every validator instantiate a template for each one.
    ///
    /// Derived from the constants rather than restated, so moving either cap fails here instead of
    /// silently reopening the hole.
    #[test]
    fn the_weight_cap_bounds_the_instructions_the_size_cap_admits() {
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            let invocations = constants.max_transaction_size_bytes / MIN_INVOCATION_ENCODED_BYTES;
            let weight = invocations as u64 * INVOCATION_FLOOR;
            assert!(
                weight > constants.max_transaction_weight,
                "{invocations} minimal invocations fit the {}-byte cap and weigh {weight}, within the {} weight cap",
                constants.max_transaction_size_bytes,
                constants.max_transaction_weight,
            );
        }
    }

    /// The block budget is denominated in weight units, and [`INVOCATION_FLOOR`] is what a weight
    /// unit means for any transaction that calls a template. Raising the floor without raising the
    /// budget shrinks the block silently: the same transactions weigh more, so fewer fit, with
    /// nothing failing to say so. This pins the command count the budget was calibrated for.
    #[test]
    fn the_block_budget_admits_the_calibrated_command_count() {
        /// Commands per block the propose budget is sized for, from the throughput measurement in
        /// the comment on `max_block_weight`.
        const CALIBRATED_COMMANDS_PER_BLOCK: u64 = 160;

        // A plain transfer, weighed by the same function consensus weighs a real one with, so every
        // term is derived: the invocation floor, the per-literal divisor, and the per-input and
        // per-signer factors alike.
        let account = ComponentAddress::new(ObjectKey::default());
        let transfer_weight = Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(account, Amount::new(2000))
            .call_method(account, "withdraw", args![TARI_TOKEN, Amount::new(100)])
            .put_last_instruction_output_on_workspace("bucket")
            .call_method(account, "deposit", args![Amount::new(100)])
            .build_and_seal(&PrivateKey::from(1u64))
            .calculate_transaction_weight()
            .as_u64();

        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            let commands = constants.max_block_weight / transfer_weight;
            assert!(
                commands >= CALIBRATED_COMMANDS_PER_BLOCK,
                "a {}-weight block holds {commands} transfers of {transfer_weight} weight, under the \
                 {CALIBRATED_COMMANDS_PER_BLOCK} it is calibrated for — raise max_block_weight with the floor",
                constants.max_block_weight,
            );
            assert!(
                constants.max_commands_in_block as u64 >= commands,
                "max_commands_in_block ({}) cuts the weight budget short at {commands} transfers",
                constants.max_commands_in_block,
            );
        }
    }

    #[test]
    fn max_transaction_weight_admits_max_template_publish() {
        // A max-size template publish carries the binary as a blob, weighing roughly
        // (binary bytes / 3) (see `calc_blobs_weight`). The per-transaction cap must stay above this
        // so honest template publishes are never rejected at ingress.
        let max_template_publish_weight = ENGINE_LIMITS.max_template_binary_size_bytes as u64 / 3;
        for constants in [
            ConsensusConstants::mainnet(),
            ConsensusConstants::devnet(7),
            ConsensusConstants::esmeralda(),
            ConsensusConstants::testnet(),
        ] {
            assert!(
                constants.max_transaction_weight > max_template_publish_weight,
                "max_transaction_weight ({}) must exceed a max-size template publish (~{})",
                constants.max_transaction_weight,
                max_template_publish_weight,
            );
        }
    }
}
