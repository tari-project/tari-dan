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

use tari_engine_types::{fees::ExhaustBurnRate, limits::ENGINE_LIMITS};
use tari_ootle_common_types::{Epoch, NumPreshards};
use tari_ootle_transaction::Network;

/// Room above a template binary for the rest of the transaction carrying it: its other instructions,
/// inputs, signatures and CBOR framing.
///
/// Deliberately loose against those — a real max-size publish encodes to a couple of hundred bytes
/// over its binary, and a single instruction is capped at `ENGINE_LIMITS.max_call_size` — because a
/// legitimate transaction refused at ingress is a worse failure than the bytes a larger allowance
/// costs. `max_transaction_size_admits_max_template_publish` is what holds it to that.
const TRANSACTION_ENVELOPE_ALLOWANCE: usize = 256 * 1024;

/// The byte cap every network uses, derived so that it moves with the template binary limit it has
/// to admit. Keeping it uniform is what lets one gossip limit serve the whole network.
const MAX_TRANSACTION_SIZE_BYTES: usize = ENGINE_LIMITS.max_template_binary_size_bytes + TRANSACTION_ENVELOPE_ALLOWANCE;

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
    /// The number of missed proposals before a node will immediately send a NEWVIEW to the next leader when the node
    /// who missed the proposals is selected as leader.
    pub missed_proposal_suspend_threshold: u64,
    /// The number of missed proposals before a EvictNode command is proposed.
    pub missed_proposal_evict_threshold: u64,
    /// The number of rounds a node must participate before their non-participation is reset. If a peer is offline,
    /// gets suspended and comes online, their missed proposal count (up to a maximum of
    /// `missed_proposal_recovery_threshold`) is decremented for each block that they participate (vote) in. Once
    /// this reaches zero, the node is considered stable and out of suspension.
    pub missed_proposal_recovery_threshold: u64,
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
    /// The share of collected fees that is burned rather than paid to leaders, in basis points. The user's price
    /// is the fee table alone; this only splits what was collected. `10_000` burns everything and leaders receive
    /// nothing. CONSENSUS RULE: must be uniform network-wide, otherwise nodes diverge on the burn totals in block
    /// headers. Use `exhaust_burn_rate` to resolve the rate for a given epoch rather than reading this field
    /// directly.
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
        missed_proposal_evict_threshold: 10,
        missed_proposal_recovery_threshold: 5,
        // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
        // transactions (~62 weight each, ~31k weight) executed in ~11.5s — i.e. ~2.7k weight/s.
        // A 10000 budget (~160 of those commands) therefore projects to ~3.7s of propose-time
        // execution, comfortably within the 10s block time and under the 5s execution circuit
        // breaker, while heavier transactions naturally consume more of the budget. The breaker in
        // on_propose is the backstop for outliers the static weight under-estimates. NOTE:
        // propose-time execution is sequential, so more cores does not raise this proportionally —
        // calibrate to single-core throughput.
        max_block_weight: 10_000,
        max_commands_in_block: 1000,
        // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
        // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
        // within the 10s block time. Rejects the ~31k-weight/500-command overload that broke things.
        max_block_validation_weight: 15_000,
        // Admits the heaviest legitimate transaction — a 1.5 MiB template publish is ~524k weight
        // (binary bytes / 3) — with ~2x headroom, while bounding any single transaction's
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
        missed_proposal_evict_threshold: 10,
        missed_proposal_recovery_threshold: 5,
        // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
        // transactions (~62 weight each, ~31k weight) executed in ~11.5s — i.e. ~2.7k weight/s.
        // A 10000 budget (~160 of those commands) therefore projects to ~3.7s of propose-time
        // execution, comfortably within the 10s block time and under the 5s execution circuit
        // breaker, while heavier transactions naturally consume more of the budget. The breaker in
        // on_propose is the backstop for outliers the static weight under-estimates. NOTE:
        // propose-time execution is sequential, so more cores does not raise this proportionally —
        // calibrate to single-core throughput.
        max_block_weight: 10_000,
        max_commands_in_block: 1000,
        // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
        // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
        // within the 10s block time. Rejects the ~31k-weight/500-command overload that broke things.
        max_block_validation_weight: 15_000,
        // Admits the heaviest legitimate transaction — a 1.5 MiB template publish is ~524k weight
        // (binary bytes / 3) — with ~2x headroom, while bounding any single transaction's
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
        exhaust_burn_rate: ExhaustBurnRate::new(500), // 5%
        max_transaction_validity_epochs: 2160,
    };
    pub const TESTNET: Self = Self {
        base_layer_confirmations: 100,
        committee_size_per_shard_group: 40,
        num_preshards: NumPreshards::current(),
        pacemaker_block_time: Duration::from_secs(10),
        missed_proposal_suspend_threshold: 5,
        missed_proposal_evict_threshold: 10,
        missed_proposal_recovery_threshold: 5,
        // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
        // transactions (~62 weight each, ~31k weight) executed in ~11.5s — i.e. ~2.7k weight/s.
        // A 10000 budget (~160 of those commands) therefore projects to ~3.7s of propose-time
        // execution, comfortably within the 10s block time and under the 5s execution circuit
        // breaker, while heavier transactions naturally consume more of the budget. The breaker in
        // on_propose is the backstop for outliers the static weight under-estimates. NOTE:
        // propose-time execution is sequential, so more cores does not raise this proportionally —
        // calibrate to single-core throughput.
        max_block_weight: 10_000,
        max_commands_in_block: 1000,
        // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
        // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
        // within the 10s block time. Rejects the ~31k-weight/500-command overload that broke things.
        max_block_validation_weight: 15_000,
        // Admits the heaviest legitimate transaction — a 1.5 MiB template publish is ~524k weight
        // (binary bytes / 3) — with ~2x headroom, while bounding any single transaction's
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
            missed_proposal_evict_threshold: 10,
            missed_proposal_recovery_threshold: 5,
            // Calibrated against 2-core hardware (Esmeralda class), where ~500 LocalOnly stress
            // transactions (~62 weight each, ~31k weight) executed in ~11.5s — i.e. ~2.7k weight/s.
            // A 10000 budget (~160 of those commands) therefore projects to ~3.7s of propose-time
            // execution, comfortably within the 10s block time and under the 5s execution circuit
            // breaker, while heavier transactions naturally consume more of the budget. The breaker in
            // on_propose is the backstop for outliers the static weight under-estimates. NOTE:
            // propose-time execution is sequential, so more cores does not raise this proportionally —
            // calibrate to single-core throughput.
            max_block_weight: 10_000,
            max_commands_in_block: 1000,
            // 1.5x the proposal budget: honest blocks (<= max_block_weight) are never rejected, while a
            // full validation-weight block projects to ~5.5s of execution on 2-core hardware — well
            // within the 10s block time. Rejects the ~31k-weight/500-command overload that broke things.
            max_block_validation_weight: 15_000,
            // Admits the heaviest legitimate transaction — a 1.5 MiB template publish is ~524k weight
            // (binary bytes / 3) — with ~2x headroom, while bounding any single transaction's
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
            exhaust_burn_rate: ExhaustBurnRate::new(500), // 5%
            max_transaction_validity_epochs: 2160,
        }
    }

    /// Resolves the exhaust burn rate in effect at the given epoch. The rate is currently a
    /// network-wide constant; the epoch parameter is the seam through which a future epoch-varying rate is
    /// introduced without touching call sites.
    pub fn exhaust_burn_rate(&self, _epoch: Epoch) -> ExhaustBurnRate {
        self.exhaust_burn_rate
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
    use tari_common_types::types::PrivateKey;
    use tari_engine_types::limits::{
        ENGINE_LIMITS,
        MAX_NATIVE_POINTS_PER_TRANSACTION,
        MAX_WASM_POINTS_PER_TRANSACTION,
        MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK,
    };
    use tari_ootle_transaction::Transaction;

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
