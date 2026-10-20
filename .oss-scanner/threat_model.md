# Threat model: Tari Ootle

## What this project does and where untrusted input enters

Tari Ootle is a sharded smart-contract layer secured by the Minotari proof-of-work chain (L1). Validators register on
L1, and each epoch the epoch manager splits the registered set into committees, each owning a range of the shard
space. A committee orders and commits transactions with a HotStuff-style BFT protocol (`crates/consensus`).
Transactions that touch substates in several shard groups are coordinated with foreign proposals. Contracts are
**templates**: WASM modules that anyone can publish, executed by the engine (`crates/engine`) under compute metering,
with fees paid in TARI. State lives in a Jellyfish Merkle tree (`crates/state_tree`, `crates/jellyfish`), and each
epoch ends with a checkpoint that the committee signs. Wallets verify the state they read against those checkpoints.

The scanned branch is `development`. The binaries that matter are:

- `tari_validator_node` (`applications/tari_validator_node`): consensus, the mempool, execution and state sync.
- `tari_indexer` (`applications/tari_indexer`): a public REST API that wallets and dapps read state, events and
  proofs from and submit transactions through. It also dry-runs transactions.
- `tari_ootle_walletd` (`applications/tari_walletd`) and the wallet SDK and light client (`crates/wallet/*`): key
  management, transaction building, stealth outputs, and verification of indexer responses.

Trust assumptions:

- **L1 is trusted.** The validator set, epochs and burn claims come from a Minotari base node the operator runs. A
  flaw that needs a malicious base node or a reorg deeper than the L1 confirmation depth is out of scope.
- **Fewer than a third of each committee are Byzantine.** Any single validator may be malicious, and so may any
  coalition below the BFT threshold. A finding that needs a third or more of a committee to collude is out of scope.
- **Every transaction and every template is attacker-controlled.** Anyone can publish a template and call any
  template with any arguments, paying only the metered fee.
- All nodes upgrade together. Mixed-version networks and state from before protocol version V2 do not exist.

Treat all of the following as attacker-controlled.

**Against a validator node:**

- Every libp2p message from any peer (`networking/`, `crates/p2p`, `applications/tari_validator_node/src/p2p`):
  gossiped transactions, consensus messages (proposals, votes, timeout votes, new-views, foreign proposals), and the
  RPC requests and responses used for catch-up sync, missing-transaction requests and state sync
  (`crates/rpc_state_sync`, `crates/validator_node_rpc`). Before the epoch check, a peer may be any libp2p node; after
  it, it may be a registered validator in the same or another committee.
- Quorum certificates, timeout certificates and checkpoint proofs carried inside those messages. Their signatures,
  signer sets and epochs must be checked.
- The body of every transaction: instructions, arguments (CBOR, `crates/tari_bor`), declared inputs, signatures,
  stealth and confidential proofs, and blobs.
- Every template's WASM code, its ABI definition, and every host call it makes at run time.

**Against an indexer:**

- Every request to its REST API (`applications/tari_indexer/src/rest_api`), including dry runs, event queries,
  substate and proof lookups, and transaction submission.
- Every response from the validators it syncs from and queries. A single validator may lie.

**Against a wallet:**

- Every response from the indexer or validators it talks to. The light client
  (`crates/wallet/light_client`) must reject state, proofs and validator-set handovers that the signed checkpoints do
  not support. It trusts the first checkpoint it sees (trust on first use) and walks forward from there.
- Transactions, manifests and template ABIs a dapp asks it to sign or run.

## Components that matter most / least

Most important, in this order:

1. **The engine** (`crates/engine`, `crates/engine_types`, `crates/template_abi`, `crates/template_lib`). A template
   must not escape the WASM sandbox, take or forge another account's resources, bypass access rules or
   ownership checks (`runtime/auth.rs`, `runtime/tracker_auth.rs`), mint or burn a resource it lacks the authority
   for, or break the invariants of confidential and stealth outputs (`engine_types/src/confidential`, `stealth`).
   Metering (`src/fees`, `wasm/bulk_metering.rs`) must charge for the work done, so that the fee bounds CPU, memory
   and state growth. Execution must be deterministic across validators. A difference in results between honest
   validators splits a committee.
2. **Consensus** (`crates/consensus/src/hotstuff`, `crates/consensus_types`). Safety (no two conflicting blocks
   commit, no block commits without a valid QC) and liveness (no single validator or transaction can stall a
   committee). This covers message validation (`on_message_validate.rs`, `on_inbound_message.rs`), vote and QC
   handling (`vote_collector`, `on_receive_vote.rs`), timeouts and leader skipping (`pacemaker.rs`,
   `leader_skip.rs`), cross-shard coordination (`foreign_proposal_processor.rs`, `on_receive_foreign_proposal.rs`),
   epoch changes (`epoch_end_drain.rs`, `epoch_state.rs`) and catch-up sync.
3. **Transaction validation** (`crates/transaction_validation`, `crates/transaction`). These checks run before a
   transaction reaches consensus: signatures, size and weight limits, input limits and dry runs.
4. **State and proofs** (`crates/state_tree`, `crates/jellyfish`, `crates/state_store_rocksdb`, `crates/sidechain`).
   A proof must not verify for a substate or version that is not in the committed state.
5. **Networking** (`networking/*`, `crates/p2p`). Unauthenticated peers must not crash a node or grow its memory
   without bound.
6. **The indexer's public API** and **the wallet's light client and stealth/confidential key handling**
   (`crates/wallet/crypto`, `crates/wallet/sdk`).

Every release binary is built with `panic = "abort"` (see the workspace `Cargo.toml`). A panic anywhere in a
validator, including in a spawned task, kills the whole process.

Lower priority but in scope: the builtin templates (`crates/template_builtin/templates`: account, faucet, liquidity
pool, burn-rate governance), the transaction manifest parser (`crates/transaction_manifest`), the C FFI
(`crates/ootle_sdk_ffi_c`) and the browser WASM build (`crates/ootle_wasm`).

Out of scope:

- `integration_tests`, `crates/consensus_tests`, `crates/template_test_tooling`, test templates under any
  `tests/templates` directory, and any code behind a test-only feature.
- Developer tooling: `applications/tari_swarm_daemon`, everything under `utilities/`, `scripts/`, `docker/`,
  `.github/` and the web UIs (`*/web_ui`).
- The ledger hardware-wallet app (`crates/wallet/ledger/app`). It needs a device SDK this image does not have.
- Flaws wholly inside `wasmer`, `cranelift`, `libp2p`, `rocksdb`, `tari_crypto`, `bulletproofs`, `ciborium` or `tokio`.
  Those projects have their own security processes. Report one here only when Ootle's use of it makes it reachable,
  and show the path.

## How to exercise it

The image is a checkout of `development` in `/src`. The workspace (minus `integration_tests`) is built with
`--all-features` in the `ci-build` profile, CI's test profile: release with `opt-level = 1` and overflow checks on.
Every crate the workspace and each standalone template crate depend on is fetched, and `CARGO_NET_OFFLINE=true` is
set, so a new template resolves its dependencies from that cache.
cargo-nextest and the `wasm32-unknown-unknown` target are installed.

**Keep the build flags.** This machine is small. Always pass the same flags so cargo reuses the built artifacts, and
select tests with a nextest filter instead of `-p`, which resolves features differently and rebuilds dependencies:

```
cargo nextest run --workspace --exclude integration_tests --all-features --cargo-profile ci-build \
  -E 'package(tari_engine) and test(/shenanigans/)'
```

**Engine.** Write a template that attacks the engine, and drive it with `TemplateTest`
(`crates/template_test_tooling`):

- Copy `crates/engine/tests/templates/hello_world` to a new directory under `crates/engine/tests/templates/` and
  edit `src/lib.rs`. Use only dependencies an existing template already uses, since the registry is offline.
- Load it with `TemplateTest::new(CRATE_PATH, ["tests/templates/<name>"])` from a test in `crates/engine/tests/`.
  `TemplateTest` compiles the template to WASM, builds and signs transactions, and returns receipts. `enable_fees()`
  turns on metering.
- Good starting points: `shenanigans.rs` (templates that misbehave), `reentrancy.rs`, `access_rules.rs`,
  `caller_badge.rs`, `limits.rs`, `metering_budget.rs`, `native_compute_budget.rs`, `confidential.rs` and
  `stealth.rs`.
- The engine and builtin-template tests ran during the image build, so every existing test template is already
  compiled.

**Consensus.** `crates/consensus_tests` runs whole committees in one process with in-memory networking, a fake epoch
manager and real execution. Start from `Test::builder().add_committee(0, vec!["1", "2", "3", "4"]).start().await`
(`src/support/harness.rs`). `peer_message_admission.rs`, `vote_equivocation_withholding.rs`,
`foreign_proposal_size.rs` and `leader_failure.rs` show how to inject malformed or Byzantine messages and assert on
the outcome. These tests run real timers. Expect seconds to minutes each.

**Transactions and codecs.** `crates/transaction_validation` and `crates/tari_bor` have unit tests that build
transactions and values by hand. The fuzz targets in `fuzz/fuzz_targets` (`transaction_decode`, `value_decode`,
`substate_id_from_str`, `wasm_validate_code`) show the decode entry points. cargo-fuzz needs nightly, which is not in
this image, so turn a target's body into a test or a proptest instead.

**Live nodes.** A validator node cannot run offline: it reads its epochs and validator set from a Minotari base node.
Reproduce findings in-process with the harnesses above.

## How you rate severity

Say which position the attacker holds: an unauthenticated network peer, a transaction sender or template author, a
single validator (or a coalition below a third of a committee), an indexer, or a validator answering a wallet.
Findings against the default configuration weigh more than ones that need a non-default option.

- **Critical**
  - Stealing, minting, forging or freezing someone else's funds or substates: bypassing an access rule, ownership
    check or proof requirement; double spending; spending a confidential or stealth output without its key; or
    breaking the balance of a confidential or stealth transfer.
  - A template escaping the WASM sandbox, or reading or writing host memory.
  - A consensus safety violation: two conflicting blocks committed, a block committed without a valid QC, or a forged
    QC, TC or checkpoint accepted.
  - A wallet light client accepting a substate, proof or validator set that the signed checkpoints do not support.
  - Memory corruption in any crate.
- **High**
  - An unauthenticated peer, a transaction or a template crashing a validator node (any panic, given `panic =
    "abort"`), hanging it, or growing its memory, disk or CPU without bound.
  - A single transaction, or a single validator below the BFT threshold, halting a committee or stalling cross-shard
    transactions for good.
  - Non-deterministic execution or validation that makes honest validators disagree on a result.
  - Metering or fees that let work grow without bound for a fixed fee, or that let a transaction skip paying.
  - The indexer crashed, or its memory, disk or CPU grown without bound, from its public API.
- **Medium**
  - Pricing that charges much less than the work it causes, but stays bounded.
  - A Byzantine validator below the threshold delaying a committee for a bounded time, or forcing extra rounds.
  - An indexer that serves wrong data a wallet then uses without detecting it, where no funds are lost.
  - A wallet, SDK, FFI or browser build crashed by an indexer or validator response, or by a manifest or ABI a dapp
    supplies.
  - Leaking a wallet's view or spend keys, stealth-output owners or confidential amounts to parties the protocol does
    not reveal them to.
- **Low**
  - Problems that need a non-default option meant for trusted setups, or a malicious local config file or
    environment variable.
  - Panics caused by invalid arguments from the embedding application of an SDK or FFI.
  - Secrets written to logs at debug or trace level.

A bug with no security consequence, such as a wrong metric, a stale doc or a test flake, is not a vulnerability. Do
not report it.

## How to report

- Give a reproducer that runs in this image, offline: a test using `TemplateTest`, the consensus harness, or the
  crate's own unit-test helpers. Include the exact transaction, template or message, and what was observed. For
  resource exhaustion, give the measured memory, CPU or time, and the fee paid if any.
- Name the attacker position and any configuration that differs from the default.
- Propose a minimal patch against `development`, with a regression test that fails without it. Keep unrelated
  refactoring out of it, and keep guards out of `template_lib` builders: they run in every template's WASM, so checks
  belong in the engine.
- Report one root cause once.

## Anything to leave alone

- The validator node's JSON-RPC on `127.0.0.1:18200` has no authentication. It is a local operator interface. A
  report must show a new way for a remote party to reach it.
- The metrics listeners have no authentication. They are meant for trusted networks.
- Template addresses include the author's public key, and a template's `metadata_hash` is free text the engine does
  not check.
- The wallet daemon narrows a manifest's inputs to the ones a dry run read. If state changes between the dry run and
  submission, the transaction aborts with `WriteToReadDeclaredInput` and the user retries. That abort is expected.
- The wallet trusts the first checkpoint it sees.
- Pre-V2 protocol behaviour and version gates for older networks. Testnets reset to the current protocol version.
