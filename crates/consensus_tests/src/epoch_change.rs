//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::{Duration, Instant};

use tari_common_types::types::FixedHash;
use tari_consensus::hotstuff::{HotStuffError, HotstuffEvent};
use tari_consensus_types::{Decision, LeafBlock};
use tari_ootle_common_types::{Epoch, NodeHeight, optional::Optional};
use tari_ootle_storage::{
    StateStore,
    StateStoreReadTransaction,
    consensus_models::{Block, BookkeepingModel},
};
use tokio::sync::broadcast;

use crate::support::{Test, TestAddress, logging::setup_logger};

async fn epoch_change(mut test: Test) {
    test.start_epoch(Epoch(1)).await;
    let mut remaining_txs = 10;

    loop {
        if remaining_txs > 0 {
            test.send_transaction_to_all(Decision::Commit, 1, 5, 1).await;
            remaining_txs -= 1;
        }
        if remaining_txs == 5 {
            test.start_epoch(Epoch(2)).await;
        }

        if remaining_txs == 0 && test.is_all_submitted_transactions_finalized() {
            break;
        }

        let (_, _, _, height) = test.on_block_committed().await;
        if height > NodeHeight(30) {
            panic!("Not all transaction committed after {} blocks", height);
        }
    }

    let vns = test.validators_iter();
    let mut histories = vec![];
    // Assert epoch changed
    for vn in vns {
        vn.state_store
            .with_read_tx(|tx| {
                let leaf_block = tx.leaf_block_get(Epoch(2))?;
                assert_eq!(leaf_block.epoch(), Epoch(2));

                Ok::<_, HotStuffError>(())
            })
            .unwrap();

        histories.push(vn.transaction_executor.get_history());
    }

    // Check that all transactions executed consistently
    assert!(!histories.is_empty());
    for history in &histories {
        for (tx_id, execution) in history {
            for inner_history in &histories {
                if std::ptr::eq(history, inner_history) {
                    continue;
                }
                let other_exec = inner_history.get(tx_id).expect("missing execution for tx_id");
                let r1 = execution.execution.result();
                let r2 = other_exec.execution.result();
                assert_eq!(r1.finalize.result.is_accept(), r2.finalize.result.is_accept());

                // NB: executions use a consistent epoch
                assert_eq!(
                    execution.execution_epoch, other_exec.execution_epoch,
                    "epoch mismatch for tx_id {}",
                    tx_id
                );
            }
        }
    }

    test.stop();
    test.assert_clean_shutdown().await;
    log::info!("total messages sent: {}", test.network().total_messages_sent());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn single_shard_epoch_change() {
    setup_logger();
    let test = Test::builder()
        .modify_config(|config_mut| {
            config_mut.epoch_end_grace_period = Duration::from_millis(10);
        })
        .add_committee(0, vec!["1", "2"])
        .start()
        .await;

    epoch_change(test).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn multishard_epoch_change() {
    setup_logger();
    let test = Test::builder()
        .modify_config(|config_mut| {
            config_mut.epoch_end_grace_period = Duration::from_millis(0);
        })
        .add_committee(0, vec!["1", "2"])
        .add_committee(1, vec!["3", "4"])
        .start()
        .await;

    epoch_change(test).await;
}

/// A validator whose base-layer oracle has not yet observed the next epoch's boundary block must
/// **not** vote for the EndEpoch block — it cannot ratify a next-epoch hash it has never seen.
///
/// This is the safety half of carrying the next epoch's hash in the EndEpoch command: voting is how
/// the committee ratifies that hash, so a node that votes without having observed the boundary block
/// would be lending quorum to a value it cannot verify. Combined with a base-layer reorg that splits
/// the committee, a permissive vote here could push a non-canonical hash to quorum and lock it — the
/// very wedge we are preventing. So the voter no-votes with `NoVoteReason::EndOfEpochHashNotObserved`
/// and waits until its (lagged, reorg-stable) oracle catches up.
///
/// The test:
///   1. Runs four validators in a single committee.
///   2. Caps validator "1"'s oracle at `Epoch(1)` so `get_epoch_hash(Epoch(2))` returns `NoEpochFound` for it only.
///   3. Drives an epoch change to `Epoch(2)`. The three observers ratify and commit the EOE (3 = quorum) and advance;
///      validator "1" no-votes the EOE (it has no Epoch(2) hash to check against).
///   4. Asserts the three observers reached `Epoch(2)` while validator "1" stayed on `Epoch(1)` with no leaf in
///      `Epoch(2)` — i.e. it never lent quorum to, or locked, an unobserved hash.
///
/// (A lagged node recovers in production via state sync once it sees an authenticated future-epoch
/// QC — see `epoch_change_no_vote_wedge_escalates_on_future_qc`. End-to-end self-healing after a
/// committee split is covered by `epoch_change_hash_divergence_self_heals`.)
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn epoch_change_unobserved_next_hash_is_not_voted() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(2);
        })
        .with_test_timeout(Duration::from_secs(60))
        // 4 validators so the 3 observers reach quorum without the lagged one.
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    let lagging_addr = TestAddress::new("1");

    test.start_epoch(Epoch(1)).await;

    // A few transactions to get the chain moving in epoch 1.
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // Cap the lagged validator's oracle BEFORE the epoch transition. After this,
    // `get_epoch_hash(Epoch(2))` returns `NoEpochFound` for this validator only, so at vote time it
    // has no next-epoch hash to ratify the EndEpoch command against.
    test.get_validator(&lagging_addr)
        .epoch_manager
        .set_oracle_visible_epoch(Epoch(1));

    // Trigger the epoch change. All four workers receive `EpochChanged(2)`. The three observers
    // ratify the EOE's next-epoch hash against their oracle and vote; validator "1" no-votes with
    // `EndOfEpochHashNotObserved`. Three votes is quorum, so the EOE still commits and the observers
    // advance — without the lagged node's vote.
    test.start_epoch(Epoch(2)).await;

    // Keep the leaders proposing so the EOE 3-chain commits on the observers.
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // The three observers advance to Epoch(2); the lagged one is excluded.
    wait_for_validators_at_epoch(&mut test, &lagging_addr, Epoch(2), Duration::from_secs(30)).await;

    // The lagged validator must NOT have advanced or locked an unobserved hash:
    //   (a) its pacemaker is still on Epoch(1)
    //   (b) it has no leaf in Epoch(2) (it never created the next genesis)
    let lagged = test.get_validator(&lagging_addr);
    assert_eq!(
        lagged._current_view.get_epoch(),
        Epoch(1),
        "lagged validator must remain on Epoch(1): it cannot ratify an unobserved next-epoch hash"
    );

    let has_committed_eoe = lagged
        .state_store
        .with_read_tx(|tx| chain_has_committed_epoch_end(tx, Epoch(1)))
        .unwrap();

    let leaf_at_2 = lagged
        .state_store
        .with_read_tx(|tx| LeafBlock::get(tx, Epoch(2)))
        .optional()
        .unwrap();
    log::info!(
        "🔬 lagged state: epoch={:?} committed_eoe={} leaf_at_2={:?}",
        lagged._current_view.get_epoch(),
        has_committed_eoe,
        leaf_at_2
    );
    assert!(
        leaf_at_2.is_none(),
        "lagged validator must NOT have a leaf in Epoch(2): it no-voted the EOE and never transitioned; got \
         {leaf_at_2:?}"
    );

    log::info!(
        "✅ gate held: validator with an unobserved next-epoch hash no-voted the EOE; the observers reached quorum \
         without it"
    );

    test.stop();
    test.assert_clean_shutdown().await;
}

/// A validator whose oracle has scanned the next epoch's boundary block, but has not yet applied the
/// `EpochChanged` event that activates it, votes for the `EndEpoch`.
///
/// This is the counterpart to `epoch_change_unobserved_next_hash_is_not_voted`: there the validator
/// cannot see the boundary and must abstain; here it can, and abstaining would be a self-inflicted
/// stall. The two gates are deliberately different questions — "has the epoch ended?" and "can I
/// verify the hash this block proposes?" — and the boundary block answers both.
///
/// The window is real on a lagging node: the scanner writes headers as it crosses them, while the
/// `EpochChanged` those scans produce queue behind epoch activations that each reassign committees.
///
/// The test caps only `current_epoch()` for validator "1", leaving `get_epoch_hash(Epoch(2))`
/// answering, and asserts every validator — including the capped one — reaches `Epoch(2)`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn epoch_change_observed_boundary_is_voted_before_activation() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(2);
        })
        .with_test_timeout(Duration::from_secs(60))
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    let unactivated_addr = TestAddress::new("1");

    test.start_epoch(Epoch(1)).await;

    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // Cap only `current_epoch()`. Validator "1"'s oracle still answers `get_epoch_hash(Epoch(2))` —
    // it has the boundary block — but `em_epoch > current_epoch` is false, so its vote can only come
    // from that observation.
    test.get_validator(&unactivated_addr)
        .epoch_manager
        .set_oracle_current_epoch_cap(Epoch(1));

    test.start_epoch(Epoch(2)).await;

    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    wait_for_all_validators_at_epoch(&mut test, Epoch(2), Duration::from_secs(30)).await;

    log::info!("✅ validator with an observed but unactivated boundary voted the EOE and advanced with the committee");

    test.stop();
    test.assert_clean_shutdown().await;
}

/// Walks the chain from the leaf of `epoch` looking for a committed `EndEpoch` block.
fn chain_has_committed_epoch_end<TTx: StateStoreReadTransaction>(
    tx: &TTx,
    epoch: Epoch,
) -> Result<bool, HotStuffError> {
    let leaf = LeafBlock::get(tx, epoch)?;
    if leaf.is_genesis() {
        return Ok(false);
    }
    let mut block = Block::get(tx, leaf.block_id())?;
    loop {
        if block.is_epoch_end() && block.is_committed() {
            return Ok(true);
        }
        if block.height().is_zero() || block.parent().is_zero() {
            return Ok(false);
        }
        block = block.get_parent(tx)?;
    }
}

/// Waits until every validator EXCEPT `excluded` reaches `epoch` on its pacemaker.
async fn wait_for_validators_at_epoch(test: &mut Test, excluded: &TestAddress, epoch: Epoch, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // Drain block events so receiver buffers don't overflow.
        let _unused = tokio::time::timeout(Duration::from_millis(100), test.on_block_committed()).await;

        let all_advanced = test
            .validators_iter()
            .filter(|v| v.address != *excluded)
            .all(|v| v._current_view.get_epoch() >= epoch);
        if all_advanced {
            return;
        }
    }
    panic!("Timed out waiting for non-excluded validators to reach {epoch}");
}

/// Waits until *every* validator's pacemaker reaches `epoch`.
async fn wait_for_all_validators_at_epoch(test: &mut Test, epoch: Epoch, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let _unused = tokio::time::timeout(Duration::from_millis(100), test.on_block_committed()).await;

        if test.validators_iter().all(|v| v._current_view.get_epoch() >= epoch) {
            return;
        }
    }
    panic!("Timed out waiting for all validators to reach {epoch}");
}

/// Reproduces the second variant of the epoch-change wedge described in the
/// `consensus-epoch-change-eoe-no-vote-wedge` investigation. PR #2104 fixed the case where a
/// validator manages to vote on the EOE but its oracle is lagging when
/// `process_end_of_epoch` looks up the next epoch's hash. This test covers the *other* entry
/// point: a validator whose oracle is lagging at vote-time, so it no-votes the EOE with
/// `NoVoteReason::NotEndOfEpoch`. Peers form quorum without it, commit, and roll over.
///
/// In production this wedges the validator permanently — its leaf stays in the old epoch,
/// catch-up sync fails because peers' leaves are now in the next epoch and refuse cross-epoch
/// requests, and the oracle catching up has no code path to retry the missed transition.
///
/// The fix gates a `NeedsSync` escalation on **cryptographic evidence**, not on wall-clock
/// heuristics: in `MessageBuffer::next`, any future-epoch Proposal/NewView whose embedded
/// 2f+1 QC validates against the future epoch's committee proves the network has rolled
/// over. The lagged validator escalates to `CheckSync` → `Syncing`, which in production
/// downloads the epoch checkpoint and resumes on the new epoch's genesis.
///
/// Test flow:
/// 1. Four validators, single committee. Start Epoch(1) and exchange a few transactions.
/// 2. Take validator "1" *network-offline* and cap its `current_epoch()` at Epoch(1). Taking it offline keeps it out of
///    the leader rotation as far as peers are concerned — the other three time-out validator "1"'s leadership slot and
///    form a TC, so the chain progresses without it. Capping `current_epoch()` ensures that *if* it ever did receive
///    the EOE proposal it would no-vote (the production failure mode).
/// 3. Trigger Epoch(2). The three honest validators commit the EOE and run `process_end_of_epoch` to advance to
///    Epoch(2). They keep proposing blocks in Epoch(2), each carrying a 2f+1 QC over Epoch(2).
/// 4. Bring validator "1" back online and clear the oracle cap (modeling a node whose network rejoined and whose
///    base-layer scanner caught up). The next Epoch(2) Proposal that the network delivers to it goes through
///    `MessageBuffer::next`, where the probe verifies the embedded QC against the Epoch(2) committee and raises
///    `HotStuffError::NeedsSync`.
/// 5. We assert on the resulting `HotstuffEvent::Failure` event — its message identifies the QC-based reason. No
///    time-based sleeps: the assertion fires deterministically when an authenticated Epoch(2) QC reaches the validator.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn epoch_change_no_vote_wedge_escalates_on_future_qc() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
        })
        .modify_consensus_constants(|c| {
            // Short pacemaker so the honest validators time-out validator "1"'s leadership
            // slot quickly and form a TC to skip past it.
            c.pacemaker_block_time = Duration::from_secs(1);
        })
        .with_test_timeout(Duration::from_secs(60))
        // 4 validators so 3 honest yes-votes are enough for quorum without the lagged one.
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    let lagging_addr = TestAddress::new("1");

    test.start_epoch(Epoch(1)).await;

    // A few transactions to advance the chain in Epoch(1) before the boundary.
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // Wedge validator "1" at the vote-time gates: the `current_epoch()` cap fails
    // `em_epoch > current_epoch`, and the oracle cap leaves it without a next-epoch hash to ratify
    // against. Both are needed — a validator that can still see the Epoch(2) boundary votes for the
    // EndEpoch on the strength of that observation alone.
    test.get_validator(&lagging_addr)
        .epoch_manager
        .set_oracle_current_epoch_cap(Epoch(1));
    test.get_validator(&lagging_addr)
        .epoch_manager
        .set_oracle_visible_epoch(Epoch(1));

    // Take validator "1" offline so the network delivers no messages to/from it. This stops
    // its (now-wedged) view from blocking leader rotation on the honest committee — peers
    // time out validator "1"'s slot, gather a TC, and the next leader takes over.
    test.network().go_offline(lagging_addr.clone()).await;

    // Trigger the epoch change. The shared `inner.current_epoch` moves to Epoch(2) and
    // `EpochChanged(2)` is broadcast (via tokio, not the test network — so even the offline
    // validator receives it). The three honest validators commit the EOE and advance.
    test.start_epoch(Epoch(2)).await;

    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // The three honest validators advance to Epoch(2); the lagged one (offline + capped)
    // cannot make progress and remains on Epoch(1).
    wait_for_validators_at_epoch(&mut test, &lagging_addr, Epoch(2), Duration::from_secs(45)).await;

    let lagged = test.get_validator(&lagging_addr);
    assert_eq!(
        lagged._current_view.get_epoch(),
        Epoch(1),
        "lagged validator's view must remain on Epoch(1) before recovery"
    );
    let leaf_at_2 = lagged
        .state_store
        .with_read_tx(|tx| LeafBlock::get(tx, Epoch(2)))
        .optional()
        .unwrap();
    assert!(
        leaf_at_2.is_none(),
        "lagged validator must NOT have a leaf in Epoch(2) before recovery; got {leaf_at_2:?}"
    );

    log::info!("✅ wedge reproduced: validator {lagging_addr} stuck on Epoch(1) while peers committed Epoch(2)");

    // Subscribe to validator "1"'s event stream BEFORE clearing the oracle cap and going
    // online, so we don't miss the Failure event the worker publishes when it raises
    // `HotStuffError::NeedsSync`.
    let mut events = lagged.events.resubscribe();

    // Oracle catches up, and the network rejoins. From now on, the honest validators'
    // Epoch(2) proposals reach validator "1" — each carries an authenticated 2f+1 QC over
    // Epoch(2). The first one trips the probe in `MessageBuffer::next`.
    lagged.epoch_manager.clear_oracle_current_epoch_cap();
    lagged.epoch_manager.clear_oracle_visible_epoch();
    test.network().go_online(&lagging_addr).await;

    // Honest validators keep producing proposals in Epoch(2); send more transactions to
    // drive them.
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // Wait for the Failure event whose message identifies the QC-based escalation. This is
    // deterministic — fires on the first authenticated Epoch(2) QC that arrives.
    let outcome = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match events.recv().await {
                Ok(HotstuffEvent::Failure { message }) if message.contains("Received valid 2f+1 QC") => {
                    return message;
                },
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => panic!("validator event channel closed"),
            }
        }
    })
    .await
    .expect("Timed out waiting for QC-based NeedsSync escalation");

    log::info!("✅ probe fired: {outcome}");
    assert!(
        outcome.contains("Epoch(2)"),
        "escalation reason should reference the next-epoch QC; got: {outcome}"
    );
}

/// Companion to [`epoch_change_no_vote_wedge_escalates_on_future_qc`]: verifies the probe
/// also escalates when the validator has fallen **multiple** epochs behind (e.g., a long
/// network partition).
///
/// Without the lifted gate, `MessageBuffer::next` discards any message whose epoch is more
/// than one ahead of `current_epoch` *before* the probe runs — so a validator stuck on
/// Epoch(1) while peers reach Epoch(3) would silently drop every Epoch(3) message and
/// remain wedged. The fix runs the probe before the discard, so the first authenticated
/// future-epoch QC trips `NeedsSync` regardless of the epoch delta.
///
/// Test flow:
/// 1. Run four validators through Epoch(1).
/// 2. Take validator "1" offline and cap its `current_epoch()` at Epoch(1).
/// 3. Drive the honest three through Epoch(1) → Epoch(2) → Epoch(3).
/// 4. Bring validator "1" back online and clear the cap.
/// 5. Assert that the probe fires on an Epoch(3) message — proving the multi-epoch path works end-to-end.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn epoch_change_multi_epoch_lag_escalates_on_future_qc() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(1);
        })
        .with_test_timeout(Duration::from_secs(90))
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    let lagging_addr = TestAddress::new("1");

    test.start_epoch(Epoch(1)).await;

    for _ in 0..2 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    // Lag and remove validator "1" before peers cross any epoch boundaries. Capping the oracle too
    // keeps it from ratifying an EndEpoch off an observed boundary, which would let it follow the
    // epoch change it is supposed to lag behind.
    test.get_validator(&lagging_addr)
        .epoch_manager
        .set_oracle_current_epoch_cap(Epoch(1));
    test.get_validator(&lagging_addr)
        .epoch_manager
        .set_oracle_visible_epoch(Epoch(1));
    test.network().go_offline(lagging_addr.clone()).await;

    // First boundary: Epoch(1) → Epoch(2). The honest three commit and roll over.
    test.start_epoch(Epoch(2)).await;
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }
    wait_for_validators_at_epoch(&mut test, &lagging_addr, Epoch(2), Duration::from_secs(45)).await;

    // Second boundary: Epoch(2) → Epoch(3). Now validator "1" is *two* epochs behind, which
    // is what the pre-fix discard logic would have masked entirely.
    test.start_epoch(Epoch(3)).await;
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }
    wait_for_validators_at_epoch(&mut test, &lagging_addr, Epoch(3), Duration::from_secs(45)).await;

    let lagged = test.get_validator(&lagging_addr);
    assert_eq!(
        lagged._current_view.get_epoch(),
        Epoch(1),
        "lagged validator's view must remain on Epoch(1) before recovery"
    );

    log::info!("✅ multi-epoch wedge reproduced: peers in Epoch(3), {lagging_addr} on Epoch(1)");

    let mut events = lagged.events.resubscribe();

    // Oracle catches up + network rejoins. From now on, the honest validators' Epoch(3)
    // messages reach validator "1". Each carries a 2f+1 QC over Epoch(3) — which would
    // have been discarded outright before the gate was lifted (3 > 1 + 1).
    lagged.epoch_manager.clear_oracle_current_epoch_cap();
    lagged.epoch_manager.clear_oracle_visible_epoch();
    test.network().go_online(&lagging_addr).await;

    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }

    let outcome = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            match events.recv().await {
                Ok(HotstuffEvent::Failure { message }) if message.contains("Received valid 2f+1 QC") => {
                    return message;
                },
                Ok(_) => continue,
                Err(broadcast::error::RecvError::Lagged(_)) => continue,
                Err(broadcast::error::RecvError::Closed) => panic!("validator event channel closed"),
            }
        }
    })
    .await
    .expect("Timed out waiting for QC-based NeedsSync escalation");

    log::info!("✅ probe fired across multi-epoch gap: {outcome}");
    assert!(
        outcome.contains("Epoch(3)"),
        "escalation reason should reference the Epoch(3) QC across the multi-epoch gap; got: {outcome}"
    );
}

/// Reproduces the base-layer-reorg committee split from the incident logs and shows the committee
/// self-heals once oracles re-converge — the property the EndEpoch-hash-in-command change buys us.
///
/// Production scenario: a base-layer reorg deeper than the confirmation depth straddled an epoch
/// boundary, so 2/4 validators' oracles reported boundary hash A for the next epoch and the other
/// 2 reported hash B. Before the fix, each node stamped and *locked* its own hash into the next
/// epoch's genesis, the committee split into two irreconcilable halves, and every cross-half
/// proposal was rejected with `InvalidEpochHash` — a permanent wedge with no quorum on either hash.
///
/// With the next epoch's hash carried in the `EndEpoch` command and ratified at vote time against
/// each voter's own oracle, the divergence can no longer be locked: an EOE proposed with hash A
/// only collects the two A-votes (and B only the two B-votes), so neither reaches the 3-vote
/// quorum. The transition simply stalls — no node advances or locks. Once the base layer settles
/// and every oracle re-converges on the canonical boundary block, the EOE is ratified by all four,
/// commits, and the committee advances together. No manual recovery, no permanent wedge.
///
/// Test flow:
/// 1. Four validators, single committee. Run a few transactions in Epoch(1).
/// 2. Split the committee's oracle view of Epoch(2)'s hash: "1"/"2" see hash A, "3"/"4" see hash B.
/// 3. Trigger Epoch(2). Assert the stall: every validator stays on Epoch(1) with no leaf in Epoch(2) (neither hash can
///    muster quorum, so nothing commits or locks).
/// 4. Re-converge every oracle on hash B (the base layer settles on the surviving chain).
/// 5. Assert recovery: all four validators commit the EOE and advance to Epoch(2).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn epoch_change_hash_divergence_self_heals() {
    setup_logger();
    let mut test = Test::builder()
        .modify_config(|cfg| {
            cfg.epoch_end_grace_period = Duration::from_millis(10);
        })
        .modify_consensus_constants(|c| {
            c.pacemaker_block_time = Duration::from_secs(1);
        })
        .with_test_timeout(Duration::from_secs(120))
        // 4 validators: quorum is 3, so a 2/2 hash split can never reach it.
        .add_committee(0, vec!["1", "2", "3", "4"])
        .start()
        .await;

    // Two distinct, non-default boundary hashes standing in for the two sides of the reorg.
    let hash_a = FixedHash::from([0x0a; 32]);
    let hash_b = FixedHash::from([0x0b; 32]);

    test.start_epoch(Epoch(1)).await;

    // Move the chain in Epoch(1) and let it settle before the boundary.
    for _ in 0..3 {
        test.send_transaction_to_all(Decision::Commit, 1, 1, 1).await;
    }
    let deadline = Instant::now() + Duration::from_secs(30);
    while !test.is_all_submitted_transactions_finalized() {
        if Instant::now() > deadline {
            panic!("Epoch(1) transactions did not finalize");
        }
        let _unused = tokio::time::timeout(Duration::from_millis(500), test.on_block_committed()).await;
    }

    // Split the committee's oracle view of Epoch(2)'s boundary hash BEFORE the transition, so the
    // EOE proposer and voters read divergent next-epoch hashes — exactly the reorg-at-boundary state.
    for addr in ["1", "2"] {
        test.get_validator(&TestAddress::new(addr))
            .epoch_manager
            .set_oracle_epoch_hash(hash_a);
    }
    for addr in ["3", "4"] {
        test.get_validator(&TestAddress::new(addr))
            .epoch_manager
            .set_oracle_epoch_hash(hash_b);
    }

    // Trigger the transition. Every worker sees em_epoch == Epoch(2) and starts proposing EndEpoch,
    // but each EOE carries the proposer's hash and is ratified against each voter's oracle: A gets
    // only the two A-votes, B only the two B-votes — never the 3 needed for quorum.
    test.start_epoch(Epoch(2)).await;

    // Let the chain repeatedly attempt (and fail to commit) the EOE. No transactions needed — the
    // pacemaker proposes the EOE on its own once past the boundary.
    let drain_until = Instant::now() + Duration::from_secs(12);
    while Instant::now() < drain_until {
        let _unused = tokio::time::timeout(Duration::from_millis(200), test.on_block_committed()).await;
    }

    // Assert the stall: because neither hash reached quorum, no node committed the EOE, advanced, or
    // locked Epoch(2). This is the key property — a divergent hash can no longer be locked unilaterally.
    for vn in test.validators_iter() {
        assert_eq!(
            vn._current_view.get_epoch(),
            Epoch(1),
            "validator {} must remain on Epoch(1) while the committee is split on the epoch hash",
            vn.address
        );
        let leaf_at_2 = vn
            .state_store
            .with_read_tx(|tx| LeafBlock::get(tx, Epoch(2)))
            .optional()
            .unwrap();
        assert!(
            leaf_at_2.is_none(),
            "validator {} must NOT have a leaf in Epoch(2) during the split; got {leaf_at_2:?}",
            vn.address
        );
    }
    log::info!(
        "✅ split reproduced: committee divided on the Epoch(2) hash, EOE cannot reach quorum, no node advanced or \
         locked"
    );

    // The base layer settles: every oracle re-converges on the canonical surviving boundary block.
    for addr in ["1", "2", "3", "4"] {
        test.get_validator(&TestAddress::new(addr))
            .epoch_manager
            .set_oracle_epoch_hash(hash_b);
    }

    // With agreement restored, the next EOE (carrying hash B) is ratified by all four, commits via
    // the 3-chain, and the committee rolls over to Epoch(2) together.
    wait_for_all_validators_at_epoch(&mut test, Epoch(2), Duration::from_secs(45)).await;

    // And every validator must have stamped the *ratified* hash (hash_b) into its Epoch(2) genesis —
    // including the former-A minority ("1"/"2"), which adopts the committee's hash from the committed
    // EndEpoch command rather than its own earlier (diverged) view. All blocks in an epoch inherit the
    // genesis epoch_hash, so reading the leaf is sufficient.
    for vn in test.validators_iter() {
        let epoch_hash = vn
            .state_store
            .with_read_tx(|tx| {
                let leaf = LeafBlock::get(tx, Epoch(2))?;
                let block = Block::get(tx, leaf.block_id())?;
                Ok::<_, HotStuffError>(*block.epoch_hash())
            })
            .unwrap();
        assert_eq!(
            epoch_hash, hash_b,
            "validator {} must have stamped the ratified hash_b into Epoch(2), got {epoch_hash}",
            vn.address
        );
    }

    log::info!(
        "✅ self-heal confirmed: after oracles re-converged on the canonical hash, the committee committed the EOE \
         and advanced to Epoch(2) with epoch_hash=hash_b on every validator"
    );

    test.stop();
    test.assert_clean_shutdown().await;
}
