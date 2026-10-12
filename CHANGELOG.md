# Changelog

All notable changes to this project will be documented in this file.
See [standard-version](https://github.com/conventional-changelog/standard-version) for commit guidelines.

## Unreleased

### ⚠️ Upgrade notes

- **Protocol V2 now also binds every substate proof to its shard**: each leaf of a block's state merkle root
  commits to its shard. No persistent network depends on V2 state roots yet: esmeralda has not scheduled V2.
- **V2 state merkle roots change, so every network started on V2 must be reset.** Any non-esmeralda network or
  local database started on 0.46.0 (igor, nextnet, stagenet, mainnet, localnet) runs V2 from genesis; wipe its
  data directory before upgrading. `check_activation_schedule` does not detect this, because the genesis
  version is unchanged.

## [0.46.0](https://github.com/tari-project/tari-ootle/compare/v0.45.0...v0.46.0) (2026-10-07)

Introduces protocol version V2, whose block headers commit to a transaction merkle root. Also fixes
burn claims, bounds per-transaction and per-block state writes, and hardens networking, the indexer
and the wallet against misbehaving peers.

### ⚠️ Upgrade notes

- **Protocol V2 from genesis on every network except esmeralda**, which stays on V1 until its V2
  activation epoch is set (TBD). Validators and indexers upgrade together. (#2804)
- **A claimed burn UTXO is minted to the claim key, not the sealer.** (#2800)
- **Transactions may persist at most 4 MiB of state**; larger ones fail. (#2821)

### Features

- **Transaction merkle root in block headers** (V2), proving whether a block finalized a
  transaction. (#2804)

### Bug fixes

- **consensus:** blocks bound the bytes they write per shard, so state sync can always progress. (#2821)
- **consensus:** the leader-skip walk saturates at the top of the height range. (#2825)
- **engine:** nested template calls share one WASM compute budget. (#2817)
- **epoch_manager:** committees are reassigned when the current epoch's hash is corrected. (#2810)
- **networking:** idle RPC sessions close and inbound connections are capped. (#2816)
- **validator:** state sync batches split by encoded size; the metrics server is split from JSON-RPC. (#2812, #2815)
- **indexer:** remaining public read routes are rate limited and checkpoint pages capped at 20. (#2823)
- **wallet:** indexer requests are bounded so an unresponsive indexer fails over, and substates
  whose value does not match their id are refused. (#2802, #2813)
- **walletd:** a burn claim's revealed fee is capped at its local price, and a transfer proof's
  change nets out the revealed amount. (#2803, #2824)

## [0.45.0](https://github.com/tari-project/tari-ootle/compare/v0.44.0...v0.45.0) (2026-10-06)

Updates the minotari dependencies to 6.1.0-pre.0 and tari_crypto to 0.24.

### ⚠️ Upgrade notes

- **V1 block IDs change.** Networks already running V1 upgrade together; esmeralda, still on V0
  until epoch 11925, is unaffected now but must run 0.45.0 before activation. (#2799)
- **Validator L1 registration and exit signatures commit to the network**, and an exit also
  commits to the activation epoch of the registration it exits. Exit payload files written by an
  older validator no longer load. (#2799)

### Other

- `chore!` — **minotari 6.1.0-pre.0 and tari_crypto 0.24.** (#2799)

## [0.44.0](https://github.com/tari-project/tari-ootle/compare/v0.43.0...v0.44.0) (2026-10-06)

Introduces protocol version V1, which commits each shard's state version in the state merkle root
and lets state sync verify streamed state incrementally. Esmeralda activates V1 at epoch 11925;
every other network starts at V1. Also tightens resource authorization and bounds several peer and
indexer inputs.

### ⚠️ Upgrade notes

- **Every esmeralda validator upgrades before epoch 11925.** V1 activates there and changes the state
  merkle root; a node still on 0.43.x forks off. (#2784)
- **A synced node below its checkpoint version must resync.** (#2795)
- **Non-fungible data updates are denied to non-owners by default.** `ResourceAccessRules::new()`
  sets `update_nft_data` to `DenyAll`; templates that relied on the open default must allow it
  explicitly. (#2786)
- **The resource auth hook runs on stealth and confidential actions.** `AuthHookCaller::template()`
  returns `Option` — `None` when a transaction instruction performed the action. (#2787)
- **Only a liquidity pool may mint or burn its LP tokens.** Pools created before this release keep
  their old rules. (#2759)
- **Wallet daemon settings take `indexer_urls`.** `settings.get` also returns the configured list.
  (#2753)

### Consensus

- `feat!` — **Each shard's state version is committed in the state merkle root under V1.** (#2784)
- `fix` — **A transaction is voted forward only once its shard groups are ready.** (#2771)
- `fix` — **Requested transactions are decoded only once the response is matched.** (#2788)

### State sync

- `feat` — **Streamed state is verified incrementally with state version proofs.** (#2796)
- `fix!` — **Synced shards end at the checkpoint version.** (#2795)
- `fix` — **Synced state that never matches the checkpoint is discarded.** (#2765)
- `fix` — **Only a signed same-group QC counts as stall-probe progress.** (#2774)

### Engine

- `fix!` — **Non-fungible data updates are denied to non-owners by default.** (#2786)
- `fix` — **The resource auth hook runs on stealth and confidential actions.** (#2787)
- `fix` — **Scalar inversion is priced as an exponentiation.** (#2794)
- `fix!` — **Only the pool may mint or burn its LP tokens**, and swaps are rejected while either
  reserve is empty. (#2759, #2763)

### Validator node and networking

- `fix` — **Transaction decode is bounded by byte size and instruction count.** (#2761)
- `fix` — **A history sync request is bounded in the epoch it may ask for.** (#2776)
- `fix` — **A peer can hold a messaging stream only for a bounded time.** (#2770)
- `fix` — **A failed RPC session start releases its per-client slot.** (#2764)

### Indexer

- `feat!` — **Substate and transaction receipt proofs are served on request** with `include_proofs`.
  (#2760)
- `feat` — **Receipts and events have a retention period**, and SQLite contention is reported as
  metrics. (#2757)
- `feat` — **The economics page reports the fees validators can claim.** (#2756)
- `fix` — **State versions accepted from sync peers are bounded.** (#2767)
- `fix` — **Checkpoint exhaust burns are bounded and overflowing economic totals refused.** (#2789)
- `fix` — **Rate limits key on `CF-Connecting-IP` or the last `X-Forwarded-For` entry.** (#2755)
- `fix` — **The UI shows transactions whose execution result validators have pruned.** (#2768)

### Wallet

- `feat` — **The wallet daemon takes several indexers and fails over between them.** (#2753)
- `fix` — **`nfts.transfer` requires transfer scope on the fee payer.** (#2785)

### Other

- `fix` — **`tari_bor` escapes text-keyed maps holding a reserved key in the JSON form.** (#2777)
- `fix` — **A `blob!` name that is not an identifier is a manifest error.** (#2766)
- `feat` — **A swarm smoke test script.** (#2782)

## [0.43.0](https://github.com/tari-project/tari-ootle/compare/v0.42.0...v0.43.0) (2026-10-01)

A security release on v0.42.0. Empty proofs no longer pass resource access rules, covenant balance
proofs and template compiles are metered, transactions declare at most 1,024 inputs, and consensus
drops malformed and unauthorised peer messages. The indexer bounds dry-runs and event queries.

### ⚠️ Upgrade notes

- **Every validator upgrades and restarts together. No reset, no data migration.** Execution and
  validation rules change without a protocol version gate, so validators on different binaries
  compute different results for the same transaction.
- **Transactions may declare at most 1,024 inputs.** Every node rejects a transaction with more at
  ingress and in block validation. The wallet selects at most 992 stealth inputs. (#2744)
- **Covenant spends cost more.** A covenant balance proof is charged native points, and a statement
  with a duplicate or out-of-range covenant claim is rejected. The wallet fee estimate does not yet
  include the proof charge. (#2739)
- **A template may hold at most 2,048 functions and 512Ki parameters and locals.** Publishing one
  over either cap is rejected, and its compile is priced by function count as well as size. A
  template already published over a cap no longer loads; the built-in templates hold 157–357
  functions. (#2747)
- **A zero-amount or empty proof is an error.** Creating one fails with `OperationNotAllowed`, and
  an empty proof no longer satisfies a `resource(..)` access rule.
- **Operators — indexer rate limits are on by default.** An indexer behind a reverse proxy must set
  `indexer.rate_limits.trust_proxy_headers = true`, or every client shares the proxy's bucket.
  (#2744)
- **Indexer dry-runs fail on a missing or down input** with a 400, as consensus does, and a rejected
  transaction is a 400 rather than a 500. (#2744)
- **Indexer dry-runs run a bounded number at a time.** `indexer.dry_run_max_concurrent_executions`
  defaults to half the cores; a dry run that waits 5 s for a slot gets a 503, and each one spends
  rate-limit tokens by the compute it used. (#2745)
- **The C FFI ABI tag is `ootle-sdk-ffi-c/17`.** Hosts must expect the new tag. (#2736)

### Consensus

- `fix` — **Foreign proposals are verified against the committee of the shard group they name.**
  (#2737)
- `fix` — **Malformed and unauthorised peer messages are dropped** before they reach consensus.
  (#2738)
- `fix` — **A validator restarted across an epoch boundary resumes its unfinished leaf epoch**
  instead of stalling at height 0. (#2746)
- `fix` — **A node serves bounded state sync only for the epochs and shards it holds**, and a
  syncing node stores a checkpoint only once its state has synced. (#2733)
- `fix` — **A replica that misses proposals catches up as soon as a later proposal shows it is
  behind**, instead of waiting out three leader timeouts. (#2743)

### Engine

- `fix!` — **An empty proof no longer counts as holding its resource.** A proof of amount 0 or of no
  token ids could pass any rule gated on the resource, including mint, burn, withdraw and owner
  rules.
- `fix!` — **Covenant balance proofs are charged and verified once per partition**, so repeated
  covenant atoms cost a cache lookup. (#2739)
- `fix!` — **A transaction declares at most 1,024 inputs.** (#2744)
- `fix!` — **A template's function and variable counts are capped and priced before it compiles.**
  (#2747)

### Indexer

- `fix!` — **Dry-runs are validated and their inputs fetched in batches**, and per-IP rate limits are
  on by default. (#2744)
- `fix` — **An event query examines a bounded number of rows**, paging with a `before_id` cursor.
  (#2742)
- `fix` — **Wildcard topic filters containing `_` match**, e.g. `my_template.*`. (#2740)
- `fix` — **Dry-runs are capped in concurrency and rate-limited by compute spent**; a client that
  disconnects mid-run pays the ceiling. (#2745)
- `fix` — **Dry-runs execute in the consensus epoch**, and `/epoch-manager/stats` reports it as
  `consensus_epoch`. (#2749)

### Wallet

- `fix` — **Seed recovery finds stealth-only keys and removes the accounts it found unused**, and
  `accounts.create` works after recovery. (#2735)
- `fix!` — **The C FFI ABI tag is bumped to `ootle-sdk-ffi-c/17`.** (#2736)
- `fix` — **Auto-claim resubmits a burn claim that is not yet claimable** until its transaction
  finalizes, and the wallet reads the consensus epoch from the indexer. (#2749)
- `feat` — **An indexer client can switch endpoints at runtime.** `IndexerRestApiClient::set_endpoint`
  and ootle-rs `IndexerProvider::set_indexer_url` let an application fail over to another indexer.
  (#2750)

### Other

- `fix` — **The swarm creates the claim key before starting any validator**, and reuses one that
  already exists. (#2741, #2752)
- `fix` — **The macOS release binaries vendor OpenSSL.** (#2751)

## [0.42.0](https://github.com/tari-project/tari-ootle/compare/v0.41.4...v0.42.0) (2026-09-30)

The testnet reset release. Every network restarts at `ProtocolVersion::V0` with a wiped state.
Metadata values become CBOR, transactions declare read or write intent per input, burn claims prove
the L1 burn output, a council sets the exhaust burn rate, and the consensus audit's lock and
certificate fixes land.

### ⚠️ Upgrade notes

- **Testnet reset: every node wipes its data before upgrading.** Every network starts at
  `ProtocolVersion::V0`; the V0 substate preimage now covers `exhaust_burn` and
  `auth_hook_updater`, the substate version is a `u64`, and the storage schemas restart with no
  migration.
- **Operators — the validator's web UI is served from its JSON-RPC port, and CORS is off.** Remove
  `validator_node.web_ui_listener_address` and `validator_node.json_rpc_public_url` from
  `config.toml`; a stale key fails to load. Set `validator_node.enable_permissive_cors = true` or
  pass `--enable-permissive-cors` to allow cross-origin browsers. (#2726)
- **Metadata values are CBOR, not strings.** `Metadata::get` returns the raw encoding: read a value
  with `get_as::<T>()` or `get_str()`, and pass `insert` any encodable value by reference. The
  engine's `std.*` event payloads carry typed amounts and addresses. (#2679)
- **Manifests write typed values as macros only.** `Amount(..)`, `Address(..)`, `SubstateId(..)`,
  `NonFungibleId(..)`, `Metadata(..)`, `PublicKey(..)`, `HexBytes(..)` and `Cbor(..)` are gone; use
  `amount!(..)`, `address!(..)` and the rest. (#2679)
- **SDK `ArgValue::Metadata` maps to `ArgValue`s, and event payload values are JSON.** A text value
  is written `{"String": ".."}`; `EventSummary::payload` and the indexer's GraphQL event `payload`
  carry each value's JSON form. (#2679)
- **Transaction inputs declare read or write intent.** `Transaction::inputs` holds
  `InputDeclaration`s, and a write to a read-declared input aborts the transaction. (#2640)
- **A burn claim proves the L1 burn output and must be signed by its claim key.**
  `MinotariBurnClaimProof` is replaced in place; a claim without the claim key among the
  transaction's signers is rejected with `AccessDenied`. (#2709)
- **A stealth statement's revealed amount names its receiver.**
  `StealthOutputsStatement::revealed_output` pairs the amount with a public key whose badge must be
  in the transaction's auth scope. (#2645)
- **L1 code template registrations are ignored.** Publish templates on Ootle instead. The node's
  global database no longer has a `templates` table. (#2725)

### Consensus

- `feat!` — **A council sets the exhaust burn rate without a release.** A builtin
  `BurnRateGovernance` component holds the schedule; the rate for each epoch is ratified in the
  end-of-epoch block header. (#2675)
- `feat!` — **The timeout certificate binds the committee's high certificate, and the block header
  commits to it.** (#2636, #2642)
- `fix!` — **A block id commits to whether the block is a dummy**, so an empty proposal no longer
  collides with its dummy. (#2694)
- `feat!` — **LocalOnly commands carry no evidence.** (#2688)
- `fix!` — **A substate's lock is resolved from the chain that granted it**, so two honest nodes with
  different branch history agree on it. Pledges and foreign conflict checks read one chain too, and
  every lock a block grants on one substate is kept. (#2711, #2716, #2720, #2722)
- `fix` — **Votes and proposals are fsynced before they are sent.** (#2721)
- `fix` — **Replicas vote on blocks with lock conflicts an honest proposer defers**, as in v0.41.4,
  so a replica whose lock view differs from the proposer's no longer stalls the chain. (#2730)
- `perf` — **The pending chain is read once per block, RocksDB point lookups are cheaper, and epoch
  and state tree GC run in small transactions.** (#2718, #2719, #2723)

### Engine

- `feat!` — **Metadata values are arbitrary CBOR**, so an address in metadata costs 32 bytes rather
  than 64 hex characters. Substate hashes change; this ships with the testnet reset. (#2679)
- `feat!` — **Metadata is indexed like the rest of a substate**: an address in it counts as a
  reference, and a bucket or proof in it is rejected as transient. (#2679)
- `feat!` — **The substate version is a `u64`**, so a hot substate cannot overflow it. (#2596, #2687)
- `fix!` — **A fee claim no longer writes the validator fee pool into the transaction diff**: the
  receipt records the claim only as a fee withdrawal, and the pool is no longer priced as storage.
  (#2693)
- `feat!` — **Transaction receipts list the substates a transaction spent**: `DiffSummary::downed`
  records each substate downed without a later version, such as a spent UTXO. Receipt hashes and
  stealth fee estimates change; this ships with the testnet reset. (#2692)
- `fix` — **A write to a read-declared input aborts when the substate is kept, not when it is
  locked.** (#2715)
- `fix` — **A burn claim must cite an L1 header from an earlier epoch**, so every validator reaches the
  same verdict; a claim citing the current epoch is not yet valid and is retried in the next one.
  (#2686, #2706)
- `fix` — **A node that cannot read its own L1 headers aborts the transaction's execution** instead
  of rejecting a claim that may be valid.
- `perf` — **One WASM instance per template is reused within a transaction, and guest memory
  reservations are pooled.** Fees are unchanged. (#2697, #2701)
- `perf` — **Transaction signatures are batch-verified.** (#2695)

### Transaction manifest

- `feat!` — **`metadata!` and `cbor!` take a JSON-shaped value with typed values nested in it**, e.g.
  `metadata!({"resource": address!("resource_..")})`. Negative numbers and the full `u64` range now
  parse, where the old `cbor!` rejected the first and turned the second into a float. (#2679)
- `feat` — **Manifests take list arguments**, e.g. `gov.set_council(1u16, [var!["k1"], var!["k2"]])`,
  and a variable passed as `"[a, b]"` is a list. `var!` may now be written directly as an argument.
  (#2717)

### Validator node

- `fix!` — **The web UI is served from the JSON-RPC port and CORS is opt-in.** A page open in a
  browser on the validator's host can no longer call its JSON-RPC. (#2726)
- `feat!` — **Each base-layer header stores the L1 `block_output_mr`.** (#2707)
- `fix` — **The state store is released at shutdown.** (#2714)

### Indexer

- `feat!` — **The Validators page marks a snapshot stale when the latest probe failed, and shows the
  failure.** `/network/stats` nests each validator's snapshot under `snapshot`, beside
  `probed_at_unix_s` and `probe_error`. (#2691)
- `refactor!` — **`/templates/cached` is removed; use `/templates/catalogue`.** The Rust client's
  `list_cached_templates` and the JS client's `templatesListCached` go with it. (#2725)
- `fix` — **Failed sync peers are forgotten when the epoch moves.** (#2689)

### Wallet

- `feat!` — **Public testnets start with an empty tTARI faucet.** Claim tTARI by burning on L1; anyone can
  refill the faucet through its `deposit` method. LocalNet keeps a funded faucet. (#2724)
- `feat` — **The web UI disables "Claim Testnet Funds" while the faucet is empty.** `accounts.get_faucet_balance`
  reports the balance, and a claim against an empty faucet fails with error code `1002`. (#2724)
- `feat` — **`resources.get` and `resources.get_many` JSON-RPC methods.** (#2700)
- `fix` — **The JS client covers every walletd JSON-RPC route.** (#2699)

## [0.41.4](https://github.com/tari-project/tari-ootle/compare/v0.41.3...v0.41.4) (2026-09-26)

Hotfix on v0.41.3 to get a testnet that stopped committing, and an indexer that could not sync,
running again. Both changes are stopgaps until the next testnet reset.

### ⚠️ Upgrade notes

- **Every validator upgrades together.** Replicas vote on blocks v0.41.3 refused, so a mixed
  committee disagrees on votes. No reset, no data migration.

### Consensus

- `fix` — **Replicas no longer refuse blocks for lock conflicts an honest proposer defers**
  (#2650's audit H3 guard). A replica's lock state after `prepare` can differ from the proposer's,
  so every replica refused blocks an honest proposer built and the chain stopped committing. The
  proposer still defers such transactions; the replica-side check returns once the two lock views
  are proven identical.

### Engine

- `fix` — **Receipts written before v0.41.0 decode again.** v0.41.0 removed the
  `SignatureVerification` fee source, so a stored receipt that charged it failed to decode and state
  sync aborted on it. The variant is restored for decoding only: it is never charged and is left out
  of `FeeSource::ALL`, so fees are unchanged.

## [0.41.3](https://github.com/tari-project/tari-ootle/compare/v0.41.2...v0.41.3) (2026-09-26)

Hotfix on v0.41.2. A `ClaimBurn` transaction with a crafted merkle proof aborted every validator
that executed it, and kept aborting them on restart because the transaction stays in the pool.

### ⚠️ Upgrade notes

- **Every validator upgrades together.** A validator still on v0.41.2 aborts when it executes such
  a transaction; a patched one rejects it. No reset, no data migration.

### Engine

- `fix` — **A claim's merkle proof is decoded with every length prefix bounded by the input.**
  `tari_mmr` preallocated the declared hash count, so a count of 2^52 aborted the node on an
  allocation failure before verification could reject it.

## [0.41.2](https://github.com/tari-project/tari-ootle/compare/v0.41.1...v0.41.2) (2026-09-23)

The engine and wallet security review release. It closes the ways a crafted template or payload
could crash a validator, a wallet daemon that let anyone enrol an admin credential, and a module
cache that trusted what it read from disk. Also: validators that stop proposing are skipped instead
of costing the network a timeout, and templates compile before the transaction that needs them.

### ⚠️ Upgrade notes

- **Coordinated upgrade required.** Leader selection and execution change in ways that cannot be
  epoch-gated, so every validator and indexer restarts on the new binary together. No reset.
- **Operators — every cached template recompiles once.** The on-disk module cache is keyed to the
  new engine fingerprint, so existing artifacts are ignored after the upgrade.
- **Wallet — a WebAuthn wallet accepts no second enrolment.** Once one credential is registered,
  `webauthn.reg_start` and `reg_finish` refuse, and the requested permissions are no longer taken
  from the caller.
- **A client holding only `settings:update` can no longer change the indexer URL.** That one field of
  `settings.set` now needs `admin`; the remaining fields are unchanged, and the web UI already holds
  `admin`.
- **`ConfidentialViewVaultBalanceResponse` and `StealthUtxosDecryptValueResponse` carry a new
  `searched` field.** Additive on the wire; Rust callers that construct either response must set it.
- **Rust API** — `WalletStoreReader::webauthn_is_user_registered` is now
  `webauthn_has_any_registration`; `WebauthnAlreadyRegisteredRequest` drops `username` and
  `WebauthnFinishRegisterRequest` drops `requested_permissions`.

### Consensus

- `feat!` — **A validator that stops proposing is skipped as leader** instead of costing the network
  a timeout each time its slot comes round. Its votes keep counting, and it gets its slot back once
  it is seen participating again. (#2652)
- `feat` — **A node withholds its vote from a proposer it caught equivocating.** (#2652)

### Wallet

- `fix!` — **Any process that could reach a WebAuthn wallet's RPC port could enrol itself as an
  admin.** Enrolment is now refused once the wallet has a credential. (#2671)
- `fix!` — **Choosing which indexer the wallet trusts now takes an administrative token.** A
  preference-level permission could decide which server the wallet believes is the chain, and the URL
  itself was unchecked. (#2673)
- `fix` — **JSON-RPC parameters are no longer written to the log.** Under the shipped log config an
  imported spending key was left in plaintext in `json_rpc.log`. (#2673)
- `fix` — **One balance-recovery request can no longer occupy a thread indefinitely.** The
  brute-force value scan is bounded whatever the caller asks for, a vault's proof count is bounded,
  and the work no longer runs on a runtime worker thread. (#2673)
- `feat!` — **A balance recovery now reports which values it searched**, so an undecryptable balance
  can be told apart from one the search never reached. `confidential.view_vault_balance` and
  `stealth_utxos.decrypt_value` both gain a `searched` field. (#2673)

### Engine

- `feat!` — **A component's owner can now replace its owner rule**, including handing ownership to
  someone else or setting it to `None`, which is final. (#2677)
- `fix!` — **`ComponentManager::get_owner_proof` returns `Option<Proof>`** and no longer panics for a
  component whose owner is not a single public key. (#2677)
- `fix!` — **An `m_of_n` rule must require between 1 and all of its requirements.** A zero threshold
  used to admit everyone and one above the count admitted no one; both are now rejected. (#2678)
- `fix!` — **A deeply nested CBOR payload can no longer crash a validator**, including one hidden in
  a published template's definition. (#2666)
- `fix!` — **CBOR integers the encoder cannot produce are rejected on decode.** (#2666)
- `fix!` — **Removes undefined behaviour in how nested calls share the runtime.** (#2667)
- `fix!` — **A corrupted or stale module cache file is recompiled instead of loaded.** (#2668)
- `fix!` — **Closes the rest of the engine review**: a multi-scalar multiplication priced by only
  one of its arguments, amount arithmetic that could wrap, and four panic sites beside the
  execution path. (#2672)
- `feat` — **Templates compile in the background** as soon as a node learns it will need them,
  instead of inside the block that first calls them. (#2660)
- `perf` — **Writing a compiled template to the disk cache no longer blocks execution.** (#2661)
- `fix` — **Module cache and engine log lines reach the engine log** instead of being dropped.
  (#2663)

### Other

- `chore` — **All open dependency advisories are closed**, and advisories now fail CI. (#2670)
- `docs` — **New guide: burning Minotari and claiming TARI.** (#2680)

## [0.41.1](https://github.com/tari-project/tari-ootle/compare/v0.41.0...v0.41.1) (2026-09-21)

The consensus audit release. It closes the ways a byzantine leader could fork a committee, stall it
or crash its replicas, and stops a peer deciding how much work a node does for it. Also: a wallet
send that could hang the daemon, a disk cache that only ever grew, and a queryable log of what a
validator saw go wrong.

### ⚠️ Upgrade notes

- **Coordinated upgrade required.** Consensus and execution both change in ways that cannot be
  epoch-gated, so every validator and indexer restarts on the new binary together. No reset.
- **Operators — the memory budget rises.** The in-memory module cache default moves from 200 MiB to
  1 GiB, taking the node's enforced budget to roughly 2.3 GiB and the RAM it asks of the machine
  from \~2.3 GiB to \~3.5 GiB.
- **Operators — two new config sections**: `templates.max_disk_cache_size_bytes`
  (default 10 GiB, and the indexer gains a `templates` section of its own) and
  `[validator_node.diagnostics]`.
- **Template authors** — a non-fungible's `data` or `mutable_data` can no longer hold a `BucketId`,
  `ProofId` or address allocation.
- **Rust API** — `WasmModuleCache::open` takes a `cap_bytes` argument,
  `RuntimeError::transient_in_component_state` is now `transient_value_in_substate`, and
  `TemplateBlob` is `MaxBytes<MAX_TEMPLATE_BLOB_WIRE_BYTES>`.

### Consensus

- `fix!` — **Fixes a possible committee split**, where a byzantine leader could get two conflicting
  branches of the chain committed. (#2635)
- `fix!` — **A byzantine leader can no longer make replicas vote for a transaction an honest leader
  would have deferred.** (#2650)
- `fix!` — **A byzantine leader can no longer crash every replica's consensus worker**, repeatedly.
  (#2651)
- `fix!` — **Substate lock checks are tightened**, closing a gap the audit found in how output
  locks are granted. (#2650)
- `fix` — **A committee member can no longer take back a vote it has already cast** to break a
  quorum that is forming. (#2649)
- `feat` — **Equivocation is recorded as evidence** when a committee member votes two ways, visible
  as a diagnostic event, a Prometheus counter and in db-inspector. Nothing acts on it yet. (#2649)
- `fix` — **A node that crashes just after voting can no longer vote again at that height.** (#2649)
- `fix` — **Only committee members can feed a node's consensus**, and a response is accepted only
  from the peer that was asked. (#2646)
- `fix` — **A peer can no longer decide how much memory and work a node spends on it** — catch-up
  responses, buffered messages and stored votes are all bounded. (#2647)
- `fix` — **A leader that is ahead of the committee can still end its view**, where it previously
  had to wait one out. (#2638)
- `refactor` — **A commit proof is now a fixed size**, small enough that a long stall cannot push it
  past what the base layer will verify. (#2643)

### Engine

- `fix!` — **Transaction-scoped ids can no longer be stored in non-fungible data**, where they would
  outlive the transaction that named them. (#2634)
- `fix!` — **Anyone may call `deposit_with_auth`**, which is the point of the method — it was locked
  to the account owner, the one caller who never needs it. (#2627)
- `feat!` — **The compiled-template caches are bounded.** The on-disk one only ever grew, and would
  have reached about 29 GiB at 10,000 templates. (#2619)
- `perf` — **Templates are compiled before they are needed**, and a cached artifact is written
  off the execution path, so a transaction no longer waits on a cold compile or a disk flush.
  (#2660, #2661)
- `refactor` — **The maximum published-template size can now be changed** without making larger
  already-published templates unreadable. (#2629)

### Wallet

- `fix` — **A wallet holding many equal-valued outputs could hang the whole daemon** while selecting
  inputs for a send. The search is now capped. (#2639)

### Validator observability

- `feat` — **A validator records its own abnormal moments** — leader failures, no-votes, consensus
  errors, sync transitions, panics — in a bounded event log, queryable over JSON-RPC and in the web
  UI. (#2644)

### Release tooling and CI

- `feat` — **Release checklists, plus a pre-tag gate and a post-tag dashboard**
  (`scripts/release_check.py`, `scripts/release_status.py`). (#2630)
- `fix` — **The release build selects packages**, which is why v0.41.0 shipped no Windows binaries.
  (#2631)
- `fix` — **A failed required build leg turns the tag run red** instead of reporting green over an
  incomplete draft. Tag builds also restore the cargo cache, riscv64 is dropped, and windows-arm64
  links again. (#2632, #2633)
- `fix` — **The swarm burns funds into the wallet daemon**, not the console wallet. (#2628)
- `fix` — **The tariswap bench stamps a distinct nonce per transaction**, so identical calls no
  longer collide on one id. (#2626)

### Tests

- `test` — **Within-epoch catch-up sync has a cucumber scenario**, covering over real networking
  what only the in-process harness covered. (#2648)

## [0.41.0](https://github.com/tari-project/tari-ootle/compare/v0.40.2...v0.41.0) (2026-09-16)

The security release. Four waves of an execution-engine audit close every fund-theft and
validator-kill finding that was open against the engine — a callee's vaults leaking into a caller's
scope, a stealth UTXO spendable more than once in one transaction, account squatting, proofs
authorising frames that were never handed them, and a dozen ways a submitter could abort every
validator that touched their transaction. Alongside them: a wallet can finally spend a balance split
across many stealth UTXOs, the indexer follows each shard group's tip instead of polling for state,
and the WASM meter starts charging for work it was letting through free.

### ⚠️ Upgrade notes

**A coordinated upgrade, not a reset.** Esmeralda takes a scheduled `ProtocolVersion::V1` activation
at **epoch 11086**, which is what lets the hashed-schema changes — a receipt's `exhaust_burn`, a
resource's `auth_hook_updater`, the block header and vote shape — pivot on an epoch boundary instead
of mid-epoch. Substates created under V0 keep hashing under V0. Two conditions: every validator is on
the new binary *before* epoch 11086 (`check_activation_schedule` refuses to start a node whose
schedule disagrees with what it has already run past), and esmeralda must never have run a
post-#2520 binary while still on V0 — a receipt committed that way is hashed with `exhaust_burn` and
no epoch-granular activation can describe it.

The stricter requirement is separate: the **ungated** engine changes — metering rates, the weight
floor, event payloads, the proof scope model, the account-ownership gate — switch at *binary
deployment*, not at an epoch boundary. Two validators on different binaries compute different
receipts for the same transaction, so the fleet upgrades and restarts together.

- `fix!` — **Proofs reach a frame only as arguments.** A top-level instruction no longer copies the
  transaction's workspace proofs into a callee's frame: a method rule is evaluated at the call
  boundary and those proofs are revoked immediately after. A template method that acts on a
  badge-guarded resource must now take the badge as a `Proof` parameter. The builtin account gains
  `_with_auth` variants of every affected method and the wallet and SDK flows are migrated onto them;
  a deployed third-party template exposing no proof-taking method needs updating before those flows
  work again.
- `feat!` — **The authorization signature cap rises from 16 to 1024**, the stealth input ceiling.
  Transactions carrying 17–1024 signatures are now admitted where they were rejected at ingress and
  in block validation, so every node must run this before any node submits one.
- `feat!` — **Publishing a template costs execution points**, charged before the compile runs at
  `140_000_000 + 2100` per binary byte — `hello_world` comes to ~0.72 TARI.
  `max_template_binary_size_bytes` drops **1.5 MiB → 1 MiB**, and a publish in the **fee
  instructions is refused outright**. Block weight budgets are re-denominated with it (a call now
  weighs at least `INVOCATION_FLOOR` = 30): `max_block_weight` 10000 → 24000 and
  `max_block_validation_weight` 15000 → 36000, holding the same ~160 commands per block and the same
  ~3.7 s of propose-time execution.
- `refactor!` — **Node eviction and `epoch_end_spread_blocks` are removed.**
  `ConsensusConfig::enable_eviction_proposal`, `ConsensusConstantsFile::missed_proposal_evict_threshold`
  and `epoch_end_spread_blocks` are gone from structs that are `deny_unknown_fields`, so **a node
  config or localnet constants file still setting any of them fails to load**. Drop the lines.
  Missed-proposal suspension and recovery are untouched.
- **Operator note — a byte cap at ingress narrows a wire parameter.** New
  `ConsensusConstants::max_transaction_size_bytes` (1.75 MiB) is enforced at ingress and
  `max_gossip_message_size` is derived from it, so gossip messages drop **2 MiB → 1.77 MiB**: a node
  on the new limit cannot decode a frame an unupgraded peer relays. Mainnet
  `base_layer_confirmations` also drops 1000 → 780, taking a burn claim's wait from ~33 h to ~26 h
  and landing the lag on an L1 epoch boundary.
- `feat!` — **Builtin event payloads shrink.** `std.resource.{create,update_nonfungible_data,
  update_metadata}` now carry an empty payload and `std.vault.{deposit,withdraw}` carry only
  `amount`. The indexer's `resource_address` event filter is derived from `substate_id` alone, so it
  matches the `std.resource.*` family and **no longer matches vault events** — a subscriber wanting
  one resource's transfers filters on the vault's `substate_id`, which is the more precise
  subscription anyway. `events.resource_address` is NULL for vault events going forward; existing
  rows are untouched.
- **What breaks on esmeralda without a reset**, all accepted as testnet costs: a deployed template
  calling the retired `EngineOp::SignatureInvoke` (discriminant `0x10` reserved), or whose binary
  lacks the `tari_tdef` custom section, since the ABI is no longer read from guest memory; a
  component whose state already holds a `BucketId`, `ProofId` or address allocation, which can no
  longer be written to; and any substate already over 1 MiB, which is rejected on touch until a
  transaction brings it back under the limit. Indexer economics accumulators are in the old fee units
  until a resync.
- **Operator note — wipe the WASM artifact cache** (`wasm_cache/`) on any node built from
  `development` since #2589. `ENGINE_FINGERPRINT` stays at `v5` while the entry header grew from 8 to
  48 bytes, so a leftover file is read with the wrong layout. It fails its new CRC and self-heals,
  but wiping is the clean move.
- `fix!` — **An underfunded transaction now aborts as `InsufficientFeesPaid`** where WASM or native
  metering exhaustion previously reported `ExecutionFailure`, so a wallet can tell "resubmit with a
  bigger fee" from "this will never work". The reason rides in `Decision::Abort(..)` and the
  prepare-vote path compares the whole decision.
- `refactor!` — **API breaks worth naming:** `tari_bor::encoded_len`/`encoded_len_with` return
  `usize` rather than `Result`; `derive_fee_pool_address` returns `Result<_, InvalidFeePoolShard>`;
  `ShardGroup`'s `Deserialize`/`Decode` go through `new_checked`, so an inverted group is a decode
  error; `GetSubstatesBatchResponse` becomes a `oneof` and `GetSubstatesBatchRequest` gains
  `include_proofs`; `RpcClientConfig` gains a public field; `IdProvider`/`ObjectIds`/`EntityIdProvider`
  take `&mut self`; `SubstateCache::read` loses its `version` parameter; `tari_ootle_wallet_sdk`
  gains a required trait method.
- **Build note — Windows ships the wallet daemon only.** `wasmer-compiler-cranelift` 7.4 is a
  `compile_error!` on Windows, so the fee table and the static template-def extractor moved down to
  `tari_engine_types` and the validator node and indexer stay Linux/macOS. Intel macOS is dropped
  from the release matrix: `tari_ootle-*-macos-x86_64.zip` and `tari_ootle-*-macos-universal.zip` are
  gone and `macos-arm64` is no longer best-effort. Windows code signing is still failing on an
  expired Azure Trusted Signing secret, which is a portal change rather than a code one.

### Wallet

- `feat!` — **A fragmented balance is spendable again.** A key-path stealth spend authorizes each
  input with its own one-time key, so spending *n* stealth inputs takes *n* signatures. At a cap of
  16 that made the signature cap — not `STEALTH_LIMITS`, which admits 1024 inputs — the binding limit
  on a multi-input spend, expressed in the wrong units and in a crate a wallet author reading the
  stealth limits would never consult. A wallet whose balance was split across more than 17 stealth
  UTXOs could not spend it in one transaction, and a coinjoin was capped at 17 participants. The cap
  is now 1024, made affordable by verifying a transaction's whole signature set as one batch: one
  multiscalar multiplication over `2n + 1` terms in place of *n* double-base multiplications, ~3x at
  the cap with the per-signature cost falling as the set grows, and **2.6x cheaper to refuse** an
  invalid set than the per-signature path it replaces. Weights are hash-derived rather than sampled,
  so a validity predicate a committee votes on does not depend on local randomness. A spend at the
  cap costs 22,628 of the 24,000 weight a leader packs, so weight — not the count — is what stops one
  monopolising a block.
- `fix` — **A transfer and its fee now share one input budget.** Selection used
  `STEALTH_LIMITS.max_inputs`, the cap on a single *statement* (1000), but what bounds a wallet is
  the per-*transaction* total (1024) — and a transfer that cannot source its fee from its own
  revealed remainder carries a second statement whose inputs come out of that same total. Both
  selections were capped independently, so a fragmented wallet could build a transaction of up to
  2000 inputs against a ceiling of 1024, which the engine refuses and ingress refuses before that:
  the wallet building what the network will not take. Now `MAX_TRANSFER_INPUTS` = 960 and a
  `FEE_INTENT_INPUT_RESERVE` of 64, held back unconditionally — whether a merged statement fits the
  fee-intent credit is only known after selection, and a selection that had already claimed the whole
  budget could not fall back.
- `feat` — **Transaction finality is driven from the indexer's event stream.** The transaction
  service learned of finality only by polling the indexer for every pending transaction every 5 s. It
  now subscribes to `/events` and queries a transaction's result the moment a `TransactionFinalized`
  notification names it. An aborted transaction writes no substate and is silent on the stream, so
  the 5 s poll stays as a backstop — but while the stream is connected it only queries transactions
  pending longer than `silent_transaction_timeout` (10 s). Commits are reported as soon as the
  indexer's stream delivers the receipt, an abort is noticed within ~10 s instead of ~5 s, at a third
  of the previous query load. `/events` has no replay, so each reconnection requests a full check.
- `fix` — **A UTXO scan could silently skip outputs** — no error, no gap in the frame sequence, and
  nothing to notice after the fact. Two independent causes in the `/utxos/stream` resume protocol.
  `state_version` is stamped per sync *batch*, so every UTXO a block touches on a shard shares one
  version and a `LIMIT n` cut landing inside a version group reported that group's version as the
  watermark — the next request's `> V` filter then dropped the rest of the group, and one busy block
  on a shard was enough. And `EndOfShard.max_state_version` came from `MAX(state_version)` over the
  whole shard, ignoring `from_epoch`, `unspent_only` and truncation, so a truncated pass jumped its
  cursor past its own undelivered remainder. Reads now end on a version boundary, and truncation is
  the server's answer (`has_more`) rather than each client's inference.
- `fix` — **A missing substate answers `NotFound`, not a general error.** A caller polling for
  something that does not exist *yet* — an account it has just created, an input whose creating
  transaction has not reached the indexer — could not distinguish that from the wallet being broken,
  and only one of the two is worth retrying. The distinction was already carried by
  `SubstateApiError`'s `IsNotFoundError` impl and simply never reached the JSON-RPC layer.
- `fix` — `X-Accel-Buffering: no` on `/events`, `/transactions/events/stream` and `/utxos/stream`;
  nginx and the proxies following its conventions buffer upstream responses by default, which on a
  long-lived SSE stream means indefinitely. `/utxos/stream` declares its negotiated content type
  (`application/x-protobuf` or `application/x-ndjson`) instead of always answering
  `application/octet-stream`.
- `feat` — `put_workspace` alias on the transaction builder.

### Indexer and state sync

- `feat` — **Follow mode: the indexer stops polling.** Each shard group holds a `sync_state` stream
  open past the validator's tip and receives transitions as they commit, so cache invalidation
  latency drops from one sync round (~60 s) to roughly **one block**. Shard groups run concurrently
  under a `FuturesUnordered`; each winds its own stream down between messages on an epoch advance,
  syncs the new epoch's checkpoints, re-resolves its committee and reopens from the cursor it holds.
  A global re-plan happens only when the set of shard groups itself changes. Keepalives re-stamp the
  watermark of every shard a stream has closed off, so a quiet shard stays served for as long as its
  validator keeps answering and `state_sync_stream_deadline` (now 600 s) is only how often a quiet
  stream is reopened.
- `feat` — **Per-request deadlines and keepalives made it possible.** Both were already per-request
  fields on the wire, but the only way to set either was `RpcClientConfig`, fixed when the session is
  created — and `RpcMultiPool` caches one client per peer, so buying a long deadline for one
  long-lived stream applied it to every ordinary read on that session. New ACK keepalive frames let
  an idle streaming response prove liveness *within* its deadline rather than extending it; before,
  a responder with nothing to send was indistinguishable from one that had died. Restoring the
  framework's dead test suite (~1,900 lines behind two commented-out `mod` lines) turned up several
  pre-existing bugs, fixed here: read timeouts that restarted on *every* frame including keepalives,
  so asking for keepalives made the client wait longer and an idle stream never timed out at all; a
  refused session reported as `Ok`; a fixed budget of twenty discarded stale frames tearing down a
  whole pooled session; and a server that could not be driven without a libp2p transport.
- `fix` — **A `sync_state` responder answers only for shards it stores, at the epoch of sending.** An
  unbounded request for shards a node no longer serves streamed no transitions — because none arrive
  for those shards any more — and then closed each shard off with a completion marker. The indexer
  takes a completion marker as evidence that a shard is level with its committee, so that silence
  read as freshness and the cache kept serving values it believed current. Follow mode makes this
  load-bearing: a held-open stream spans epoch boundaries, and "peer went quiet" must read as
  *unknown*, never as *caught up*. A peer-attributable failure now skips only that shard group rather
  than costing every other group its state sync.
- `fix` — **Three ways a stale value could be installed as the live head**, all closed. A committee
  member that is behind answers with a version this indexer has already watched the substate pass —
  legitimately, from its own point of view — and the transition had already deleted the cached row
  that would have ranked the answer down, so a write below the version the stream has shown is now
  refused. A destroy with no successor no longer admits an `Up` at the destroyed version while still
  admitting a `Down` at it. And a finalized transaction result retires what it created or destroyed
  ahead of the stream, which is what the `transfer.feature` "substate does not exist" failures were:
  the wallet asked whether a destination account existed, that nonexistence was cached, it learned
  the transfer committed, and read back the stale answer.
- `feat` — **Substates the committee agrees do not exist are cached.** `DoesNotExist` is the most
  expensive lookup the indexer makes: `Up` and `Down` return on the first acceptable response, but
  absence has nothing to prove against the state tree, so it is settled by `f + 1` agreement and
  walks that many committee members — every time, because it was also the one result the cache would
  not keep. Recorded as a row with no version rather than a sentinel, so `Option`'s own ordering
  gives the head-ranking rule for free. Served only while the shard's stream is demonstrably alive
  (`state_sync_keepalive_interval × 3`), because a nonexistence is correct at the instant it is taken
  and false ever after; transaction receipts are excluded, since they are the bulk of the stream by
  count and are answered from the indexer's own tables anyway.
- `feat` — **Committee members are raced on a substate read**, up to `READ_RACE_WIDTH = 3` in flight,
  settling on the first response that decides the read. Members were asked one at a time, so an
  unreachable first pick waited out the full 10 s connect timeout — on a small committee, a 1-in-n
  chance per read of a multi-second stall. The decision rules move into one `CommitteeReadTally`, and
  `DoesNotExist` now settles at `f + 1` throughout rather than needing `f + 2` on the way through the
  loop.
- `feat!` — **The batched substate read path carries proofs.** `get_substate_batch` streamed bare
  values, so every batch result was written unverified — and with proof verification on the read path
  refused those entries and refetched singly, so the batch populated a cache it could not then use.
  `SubstateProofGenerator` hoists the parts that do not vary per substate out of the loop: 50
  substates across 50 distinct shards costs ~449 µs against ~10.68 ms one-shot, which also means one
  batched request is ~25x cheaper for a validator to serve than the 50 single reads it replaces. Two
  responder defects fixed with it — it never checked it stored the requested ids, and `missing` was
  logged and dropped rather than put on the wire.
- `refactor!` — **The cache serves only the head.** The indexer is a gateway to the network's current
  state, not to its history, so a lookup naming a version is only ever asking whether that version is
  still current; what the head says about it is decided once, in `SubstateCacheEntry::answer_at`.
- `feat` — Cache refusals, invalidations and evictions are logged and counted. A read refused because
  the shard's watermark was missing or stale came back as `Ok(None)`, indistinguishable from an empty
  cache, so an indexer whose every read had started costing a committee round trip gave no hint why.
  `api_sse_connections_active` gauges streaming connections per endpoint — the principal load signal
  from wallets now that the daemon follows `/events` — and `api_http_response_body_size_bytes` was
  constructed and observed but never registered, so it was exported nowhere.
- `fix` — A shard group with no committee answers 503 naming the group rather than a masked 500, and
  a spent version answers 404 rather than 500. A `resource_address` entry in a *template's* event
  payload can no longer make an event match a resource filter.
- `fix` — **State is served from the current committee and validated against the previous.** Only the
  quorum that makes an `EpochCheckpoint` valid needs to be the previous committee; using it to pick
  serving peers preferentially targeted validators that may have left the register, while continuing
  members of the same shard group hold identical state and are `Running`. `SyncSource` orders
  continuing members, then members that joined at this epoch, then departed prev-only members as the
  full-turnover fallback, each tier shuffled.

### Execution engine — security

Four audit waves. Every item is reachable by an ordinary submitter unless stated, and each has a
regression test verified to fail on the pre-fix code.

- `fix!` — **A callee's vaults no longer leak into the caller's scope.** `include_owned_in_scope`
  seeded a callee frame with every substate reachable from the component's state, and
  `update_from_child_scope` extended the caller's owned set with the callee's whole set on pop. So a
  template could call any `allow_all` method on a victim (`Account::get_balances`), come away with
  the victim's vault in its own scope, and `withdraw_all()` — TARI's withdraw rule is `AllowAll` — or
  persist the victim's vault id into its own state permanently. Component-reachable substates now
  live in a separate `component_owned` set, in scope for that frame alone. Buckets and proofs no
  longer merge upward either: a frame hands back only what it names in its return value.
- `fix!` — **A stealth UTXO could be spent more than once in one transaction.** `WorkingStateStore`
  kept no spent set its own reads consulted, so `exists()` answered from the immutable input snapshot
  and a downed UTXO still read as present. Three spends followed: one UTXO listed *n* times in a
  single statement — the inputs fold positionally into the excess, so a spender who knows the mask
  builds a valid balance proof for *n·v* — up to 65 statements over one UTXO in one transaction with
  one down and *k* output sets, and spend-then-`StealthUtxoBurn`, whose diff carried both a down and
  an up@v+1 of the same address.
- `fix!` — **Proof access was unscoped, and `SetVaultFreeze` froze any vault.** Proof ids come from a
  transaction-wide counter, so a component called by a frame holding a proof could name it by id and
  authorize with a badge it was never handed, or read its amount, resource and non-fungible ids.
  Separately, `SetVaultFreeze` authorized `Freeze` against a resource and then write-locked any vault
  id without checking what it held — so anyone could publish a resource with
  `freezable(rule!(allow_all))` and freeze any vault on the network, a TARI vault included, leaving
  its owner unable to withdraw or pay fees. Network-wide, and cheap.
- `fix!` — **Attacker-reachable panics are removed from execution.** The release profile is
  `panic = 'abort'` with `overflow-checks = true`, so each of these stopped every validator that
  executed the transaction: `schnorr_verify` on `PublicKey::Zero`; guest-controlled pointer
  arithmetic at publish (a ~50-byte module declaring `_ABI_TEMPLATE_DEF` as `-1`); resource balance
  overflow, including the locked/unlocked pair that bounding only the unlocked field left open; and
  `FeeBreakdown::add`. Two more in the same file: `lock_all`'s `Confidential` arm copied the revealed
  amount where it moved it, so taking and dropping a proof over a confidential vault **doubled** its
  revealed balance; and its `Stealth` arm returned the wrong container type, so every proof over a
  stealth vault — TARI included — aborted the transaction. A workspace-wide
  `clippy::arithmetic_side_effects` sweep (962 hits, every one read in context) closed the rest,
  including two wallet daemon RPCs that could crash the daemon (`claim_fees` with `shards: [0]`) or
  hang it allocating ~4 billion shard ids (`get_fees` with an inverted shard group).
- `fix!` — **Template modules are validated before they are instantiated.** `finalize_loaded_module`
  instantiated an untrusted module and validated it afterwards, and the legacy ABI path read a
  template definition out of guest-controlled linear memory — both on the `PublishTemplate` path.
  Table types were passed through untouched, so `table.grow` with a delta of `0xFFFFFFFF` asked the
  host for tens of gigabytes at a cost of two metering points. Also: `tari_alloc` and `tari_free` are
  now inside the metering window, return values are capped at `max_call_size`, and an operator the
  metering table does not price costs 1000 rather than 1.
- `fix!` — **Limits the engine documented but did not enforce on every path.** `check_write_allowed`
  now lives in a `try_lock` helper every lock passes through (five sites took a write lock without
  it); `max_substate_size` binds on mutation as well as creation, so a vault's non-fungible id set
  can no longer grow past 1 MiB one deposit at a time; and a new `max_event_size_bytes` (2 KiB)
  bounds an event payload, which the receipt could not reject for being oversized because it is built
  after fees settle. Transient ids are also rejected in component state — a `BucketId`, `ProofId` or
  address allocation written there reaches the ledger where it can only alias an unrelated object of
  a later transaction, and `invoke_resource_access_hook` passes component state into the hook frame
  as an *argument*, where a tagged proof id is treated as an authorizing proof.
- `fix!` — **Resource and proof lifecycle corrections.** A bucket whose funds a proof has locked can
  no longer be consumed by `Bucket::join`, `PayFee::FromBucket` or the stealth-transfer
  revealed-funds path. `GetOwnerProof` works on a component created earlier in the same transaction.
  A resource's auth hook can read the resource it guards — the write lock is released across the hook
  call. `validate_finalized` tests confidential vaults with `has_locked_funds()` rather than
  `locked_balance()`, which reports zero for hidden amounts.
- `fix` — **The workspace is deterministic and lock scope is enforced.** `Workspace.items`,
  `Workspace.proofs` and `address_allocations` were hash collections iterated on paths that reach
  consensus. Separately, `CallScope`'s lock-scope methods had no callers, so `DanglingSubstateLocks`
  was unreachable and a callee could leak a lock and grief the rest of the caller's transaction; the
  one place the engine really leaked a lock is fixed with it, so two `ClaimValidatorFees` against one
  pool in a transaction now succeed.

### Execution engine — pricing and metering

- `perf!` — **Engine responses cross the WASM ABI as their encoding, not a `Value` tree.** The host
  built the tree with an encode-then-decode and the guest tore it down with another, on a path where
  the guest's half is metered. Carrying `tari_bor::RawCbor` instead: a tariswap swap **−53%** guest
  metering points, an account balance read −49%, account create + fund −23%. Points are user fees, so
  that is a direct cost reduction. The wire does not move and a published template keeps working
  unchanged; only a recompiled one gets the saving. This is also what rejected the original zero-copy
  (rkyv) plan — per-byte marshalling turned out to be near free, and the cost was per call.
- `feat!` — **Bulk memory and table operators are charged by length.** `memory.copy`, `fill` and
  `init` were priced at a flat 2–4 points regardless of the length operand, so the meter bounded a
  template's instruction count and not its work — on the order of 70 TB of `memory.copy` fit the 250M
  per-transaction budget. A `BulkMetering` middleware emits an inline length-proportional charge
  against the same metering global before each bulk operator runs: 1 point per byte, 16 per table
  element.
- `feat!` — **Template instantiation is priced per data-segment byte, not per binary byte.** Compiled
  code is laid down once at publish; the faucet (151 KiB), liquidity pool (322 KiB) and account
  (530 KiB) all instantiate in the same ~0.015 ms, so pricing off `code_size()` would have
  overcharged the account template by **18x** on every instruction of every transaction.
  `memory.grow` is repriced with it — wasmer grows by mapping and the zeroing is the OS's, lazily on
  first touch, so the work is per *call* — and the builtins now declare the memory they use rather
  than taking a grow per instantiation for a page they always need. A faucet claim's WASM points:
  272,989 → 94,321 → **76,264**.
- `feat!` — **Native intrinsics.** Templates had no hashing at all and no group or scalar arithmetic,
  so anything cryptographic had to be compiled to WASM at roughly ten times the native cost — a
  Groth16 verification measured at ~112M metering points against ~9.9M natively. Ristretto and scalar
  arithmetic, MSM, four hash functions with a `_parts` variant for Merkle walks, and Schnorr
  verification. Every intrinsic is a pure function of its arguments, which is what lets the engine
  price one *before* running it; they are addressed by a permanent numeric `IntrinsicId` behind a
  single op, so adding one later changes no wire type and leaves every published template working.
  Signature verification moves onto them, from a flat 10 µT fee charge and zero metering points to
  `NativeExecutionPoints::PER_SCHNORR_VERIFY`, counting against the block execution budget.
  *The prices are provisional* — derived from the existing `PER_INPUT` calibration, not measured.
- `feat!` — **The exhaust burn is a share of what was collected, not a surcharge on top.** The fee
  table alone is now the user's price; the rate splits what was collected (`B = ⌊F·s/10_000⌋`, every
  network at 500 bps). `FEE_ESTIMATE_ALLOWANCE` drops 25 → 12, `FeeReceipt` gains `exhaust_burn`, and
  `FeeSource::ExhaustBurn` becomes `Reserved` at the same index with a JSON alias so existing
  receipts still render.
- `fix!` — **A receipt's `exhaust_burn` is hashed only from protocol version 1.** The field was added
  with `#[cbor(default)]` so old receipts decode, but the substate hash preimage is borsh and the
  derive writes every field — so a pre-change receipt re-hashed with an extra `0` that the committing
  node never covered, and any node deriving a receipt hash from its value disagreed with the
  quorum-signed state roots. Found by resyncing a validator across a localnet upgrade: 179 of 380
  receipt leaves hashed differently and a fresh validator could not sync at all.
- `perf` — Thread-safe primitives are dropped from the strictly single-threaded execution path,
  taking a heap allocation off every call-frame push.

### Execution engine — authorization

- `feat!` — **Caller identity is a badge, as in Radix.** It was a predicate evaluated once at method
  entry, so "resource R may only be withdrawn while executing on behalf of A" was inexpressible — the
  gate had to be replicated on every holder of R and was consumed at the door. Two virtual resources
  are reserved and stamped into a callee's scope by `push_frame`, so the identity holds for the
  frame's lifetime and is checkable at every auth point, resource rules included. Never inherited,
  not capturable as a `Proof`, and the addresses are unforgeable. Three vulnerabilities were fixed
  with it: hook frames run in `FrameWriteMode::OwnComponent`; a pushed frame inherits its parent's
  write mode, closing a sandbox escape; and `Recall` must match the vault's resource, where
  `recallable(allow_all)` could previously drain a vault of any other resource.
- `feat!` — **Account ownership is gated on the signer badge.** `CreateAccount` derived the canonical
  address for any public key and forwarded a caller-supplied `owner_rule`/`access_rules`, so a
  squatter could create the victim's account ahead of them, name itself the owner, and collect every
  subsequent deposit — including the ones the victim's own senders make. Custom rules now require
  that key's signer badge, and `Account::create` is no longer reachable as a `CallFunction` or
  through `TemplateManager::call`. Creating an account on the default rules stays permissionless, so
  deposits to an account that does not exist yet still work.
- `feat!` — **A resource's auth hook can be replaced or removed.** The hook was fixed at creation and
  runs on nearly every resource action, so one that panics, denies unconditionally or fails to decode
  its arguments took the whole resource offline and made the balances in its vaults unspendable —
  with no recovery at all if its component was created under `OwnerRule::None`. `ResourceAccessRules`
  gains `auth_hook_updater`, defaulting to `UpdateRule::Locked`, so today's immutability is preserved
  for every existing resource. The hook being replaced is deliberately not invoked.
- `feat!` — `caller_component(addr)` / `caller_template(addr)` rule requirements land first (#2503),
  and proof-taking `_with_auth` account methods ship ahead of the proof-scope change so call sites
  could migrate before the protocol flipped.

### Consensus

- `fix` — **A leader's proposal is anchored entirely at the state anchor.** When a leader fills a
  timeout gap with a dummy chain the candidate extends from the justify block, but the proposal
  batch, the pool query, foreign proposal selection, the change set and command generation still read
  at the highest seen block — an orphan the candidate abandons, whose pool records carry stages no
  replica reproduces. In the same class, a `drain(..)` was discarding every pending pool update and
  foreign pledge the proposer had just recorded, so the leader committed to a fee no replica
  computed: nobody voted, the next leader rebuilt the identical block from the same still-pending
  inputs, and the committee wedged at `high_qc = NodeHeight(6)` until nextest's 600 s kill.
- `fix` — **A foreign proposal that fails to process while proposing is dropped and rejected.** The
  atom had already shipped into `commands` before processing, the failure arm only logged, and
  nothing recorded the proposal as proposed — so the next leader selected it again, the same
  deterministic failure repeated, and another unvotable block shipped, with no self-clearing path.
  Only a *validation* failure condemns a proposal; a storage or epoch-manager error retries.
- `fix` — **A validator whose shard group changes can actually sync.** The refusal to open the next
  epoch was an `Err` returned from inside the write transaction that had just saved the epoch
  checkpoint, so the checkpoint was rolled back on every member on every attempt — and `check_sync`
  then answered `UpToDate`, because a node whose shard group moved is behind by neither height nor
  epoch. Sixty seconds of a six-validator swarm splitting into two committees, with the epoch never
  advancing and the joining validator never getting its checkpoint.
- `fix` — **Justified-block evidence is re-recorded on the surviving branch.** The "already
  justified" guard was a persisted per-block flag stamped by the first block to justify it, certified
  or not — so when that branch was abandoned the evidence went with it, and a multi-shard transaction
  sat at `LocalAccepted` with `is_ready=false` forever while its shard group proposed empty blocks.
- `fix!` — **Unauthenticated peers can no longer abort or restart the consensus worker.** One
  Proposal with `justify.height = u64::MAX` plus a timeout certificate aborted every validator that
  received it, and three handlers propagated errors reachable from unauthenticated gossip into the
  worker's fatal catch-all, dropping it into `Failure → Sleeping (5s) → Initialising` on demand.
- `fix` — **An `EndEpoch` hash is ratified from an observed boundary, not an activated epoch**, so a
  node that has scanned the boundary block but is behind on applying epoch activations stops
  no-voting. A deferred end-of-epoch now also resumes on the worker's 10 s periodic tick rather than
  waiting for a whole scan to complete. Sync-class errors are no longer published as consensus
  failures.

### Node, networking and swarm

- `fix` — **Connections that stop answering pings are closed.** `libp2p-ping` reports failures and
  leaves the connection open — the policy decision belongs to the user, and we were not making it —
  so a connection that could no longer carry traffic was held until the kernel exhausted
  `tcp_retries2`, roughly 15 minutes during which every message routed over it was silently lost. Hit
  on a local swarm when a VPN interface was torn down: one validator lost its path to three of six
  peers, missed every proposal those peers led, and saw-toothed behind the chain for sixteen minutes
  — *while holding healthy connections to the same peers* that traffic never used, because
  `obtain_message_channel` returns the existing sink. Default 3 consecutive failures.
- `feat` — **The validator node's memory ceiling drops 3.2 GiB → 2.4 GiB.** ~1.4 GiB of the old
  figure was RocksDB and libp2p defaults: nine column families each with their own 64 MiB × 2
  memtable budget and 32 MiB cache, and a gossipsub send queue of libp2p's default 5000 messages
  against a 2 MiB message size. Now one shared RocksDB budget with memtables charged to the same
  cache, explicit gossipsub bounds, a startup check that logs the budget table and warns when
  `MemAvailable` is short, and metrics reporting capacity and live usage at scrape time. New
  `tari-vn-bench` grades a candidate machine on whether it can keep *voting*, not just start, and
  adds a requirements page with disk capacity and bandwidth marked **not established** rather than
  guessed.
- `fix` — A block's WASM execution points are reported in the web UI; `transactions_finalize_all`
  deleted the reverse index as it finalized, so every block a user can actually look at summed to 0.
  The WASM cache entry header is CRC-checksummed and the tempfile fsync'd before the rename — the
  header sits outside the wasmer artifact and its four shape counts are consensus inputs, so
  corruption confined to it yielded a divergent receipt with no error raised anywhere.
- `feat` — Swarm log files are paged through mmap-backed byte windows instead of being fetched whole
  on each poll, which locked up or killed the tab on a long-running swarm. A process that exits
  non-zero prints its panic message to the daemon's own stdout, logs are attributed by `InstanceId`
  rather than by longest-matching path prefix, and the console filters at `Info` while `swarm.log`
  keeps `Debug`.

### Build, CI and docs

- `chore` — wasmer 7.1 → 7.4 (Cranelift 0.129 → 0.135), unblocked by bumping the libp2p fork, which
  removed the `wasm-bindgen` pin that made 7.2+ unresolvable. Consensus impact verified against the
  diff: no new `Features`, and the metering operator cost path is byte-identical. wasmer was capped
  at `~7.1.0` first, because `^7.1.0` meant published `tari_engine` 0.39.3 and 0.40.0 did not compile
  against a fresh resolve.
- `ci` — Docker images build on tags only; every merge to `development` was kicking off a full
  multi-binary image build and GHCR push that nothing consumes. The nightly binary build sheds three
  pieces of dead configuration and moves to an off-peak cron, after scheduled runs drifted from
  ~1.7 h to ~4.5 h behind their slot.
- `test` — Several CI flakes fixed at the cause: validators no longer promote themselves to `Running`
  before their peers exist, the transaction service tests wait on events rather than the clock, and
  the harness timeout is a wall-clock bound rather than one every ignored event reset — which is why
  a livelock burned 600 s and produced a 14 GB job log instead of panicking at its declared 60 s with
  the dumps the code already emits. Plus a committee-split integration scenario, enabled by a LocalNet
  consensus constants file: nothing had exercised a committee change before, because at the devnet
  default committee size of 7 a second shard group needed fourteen validators.
- `docs` — A new **Concepts** section: ten pages covering architecture, consensus, state and
  execution, privacy, stablecoins, templates and assets, tokenomics and a glossary, with sixteen
  hand-authored inline SVG diagrams. Every factual claim cites the source path it comes from, and
  RFCs and TIPs are explicitly *not* treated as authoritative where they disagree with the code. Plus
  a claim burn guide, a vulnerability disclosure policy, and corrected fee, version and WASM guidance
  across all ten agent skills — the old publish-fee guidance was unachievable for any template, since
  `per_template_publish_cost` alone is a flat 250,000 µT.

### Crate versions

`[workspace.package].version` moves to `0.41.0` (the whole tier-3 cohort). Independently versioned
crates affected:

| crate | version |
|---|---|
| `tari_bor` | 0.15.0 → 0.16.0 |
| `ootle_serde` | 0.5.0 → 0.6.0 |
| `ootle_byte_type` | 0.12.0 → 0.13.0 |
| `tari_ootle_address` | 0.10.0 → 0.11.0 |
| `tari_template_abi` | 0.19.1 → 0.20.0 |
| `tari_template_lib` | 0.31.0 → 0.32.0 |
| `tari_template_lib_types` | 0.31.0 → 0.32.0 |
| `tari_template_macros` | 0.22.1 → 0.23.0 |
| `tari_ootle_template_metadata` | 0.11.0 → 0.12.0 |
| `tari_ootle_template_build` | 0.11.0 → 0.12.0 |
| `tari_indexer_client` | 0.41.0 → 0.42.0 |
| `ootle-rs` | 0.22.0 → 0.23.0 |
| `ootle_ledger_client` | 0.6.0 → 0.7.0 |
| `tari_ootle_wallet_crypto` | 0.42.0 → 0.43.0 |
| `tari_ootle_wallet_sdk` | 0.42.0 → 0.43.0 |
| `tari_ootle_wallet_storage_sqlite` | 0.42.0 → 0.43.0 |
| `tari_ootle_walletd_client` | 0.42.0 → 0.43.0 |

`crate_versioning.py list` and `impact` now read the crates.io sparse index, so a crate whose in-tree
version was never published is reported as already covered rather than as a phantom cascade.

## [0.40.0](https://github.com/tari-project/tari-ootle/compare/v0.39.3...v0.40.0) (2026-09-02)

Two production incidents on esmeralda are fixed here — a state-sync off-by-one that corrupted the
state tree and permanently wedged a validator, and the pool-clear that kept the same node out of
consensus for an epoch. Alongside them: the WASM metering ceiling is raised to make non-trivial
cryptography viable in templates, the substate schema activation schedule becomes per-network, and
the indexer's substate cache is invalidated by the transition stream instead of a two-second timer.

### ⚠️ Upgrade notes

- `feat!` — **All validators must be upgraded together.** `max_block_validation_execution_points`
  moves from 7.1e9 to 7.25e9 and `MAX_WASM_POINTS_PER_TRANSACTION` from 100M to 250M. These are
  consensus rules requiring network-wide uniformity — two validators on different versions disagree
  on block validity.
- `fix!` — **Reject reasons change, so transaction receipts change.** A transaction aborting because
  an input has no live version now reports `"Substate {id} is not found or DOWN"` instead of
  `"Substate {id} is DOWN"`. The receipt is a substate and is hashed into the state tree, so mixed
  versions commit different receipts for the same transaction and diverge.
- **Operator note** — a validator node now **refuses to start** if its binary introduces a substate
  schema activation at an epoch the node has already run past, since that would silently re-hash
  committed substates. Override with `--allow-past-protocol-activation` (or
  `validator_node.allow_past_protocol_activation`) only for a node whose state is being discarded.
  No existing network is affected: every schedule is still `[(Epoch(0), V0)]`.
- **Operator note** — **indexer config**: `latest_substate_cache_ttl` is removed, replaced by
  `substate_cache_max_serve_lag` (default 300s) and `substate_cache_max_entries` (default 100k). The
  internal `DEFAULT_CACHE_TTL` moves 300s → 900s and is demoted from a correctness mechanism to a
  coarse backstop for entries the transitions never reach.
  `max_serve_lag` must comfortably exceed a *full* sync round
  (`state_scanning_interval` plus the time to sync every shard group), not just the interval, or the
  cache closes between rounds. A SQLite migration runs on first start; the old `cacache` directory
  is orphaned and can be deleted.
- `feat!` — **`tari_indexer_client`**: `GetIndexerInfoResponse.latest_substate_cache_ttl_secs` is
  renamed `substate_cache_max_serve_lag_secs`.

### Consensus

- `fix` — **A validator whose pool was cleared can no longer be locked out of consensus for the rest
  of the epoch.** After a state sync, `Syncing::on_enter` clears the transaction pool; the next
  epoch's block re-proposed a cleared transaction, and `evaluate_local_only_command` no-votes any
  block whose transaction is not in the pool. The node fell behind and failed the state merkle root
  check on every subsequent block — ~65 minutes out of consensus on esmeralda. Parking does not
  cover it: the transaction *record* was never missing, only the derived pool record, and there is
  nothing to request from a peer. The readiness check now widens from "which transactions do we not
  have?" to "which are not ready?", re-sequencing a held transaction through the same
  `validate_new_transaction` path used for peer-fetched ones. That path already refuses to sequence
  an id whose finalized decision is a commit or whose `TransactionReceipt` exists in state, so a
  repair cannot resurrect what the synced state committed.
- `fix!` — **A destroyed substate and one that never existed report as the same error.** Telling them
  apart is a property of how much history a node retained, not of the ledger, but the distinction was
  stringified into the reject reason and hashed into the state tree — so two honest nodes with
  different `epoch_history_length` settings committed different receipts for the same transaction.
  `SubstateIsDown` is folded into `SubstateNotFound` on both `SubstateStoreError` and
  `LockFailedError`. Nothing depended on the distinction: `try_lock_all` already classified both as
  hard conflicts. This also removes a trap — `is_not_found_error()` never matched `SubstateIsDown`,
  so merging the message without merging the variant would have turned an ordinary stale submission
  into a fatal error propagating out of consensus.
- `feat` — **The substate schema activation schedule is per network.** Networks run at independent
  epochs, so one hardcoded table cannot express when a schema goes live on each.
  `ProtocolVersion::activations` matches exhaustively on `Network`, so a new network must state its
  own schedule rather than silently inherit one; `at`, `newest_scheduled_activation` and
  `hash_substate` take a `Network` alongside the `Epoch` already threaded to every hashing site. The
  duplicate `ProtocolVersion` in `common_types` is removed and re-exported from `engine_types`, so
  the copy gating consensus and the copy gating substate hashing can no longer disagree. The
  unsatisfiable `MAX_SUPPORTED` guard in the hotstuff worker is removed — it compared two values read
  from the same binary's table.

### State sync

- `fix` — **The state stream starts at the first *unpersisted* version.** It opened at the shard's
  already-persisted version, and the stream is inclusive, so the peer replayed a version the client
  had already written. JMT nodes are keyed by `(version, nibble_path)`: the rewrite overwrote the
  live nodes at those keys while recording those same keys as stale at that version, and an hour
  later the stale-node GC deleted them out from under the current tree. The node then failed every
  block evaluation and every subsequent sync on `A node v6384:f9 expected to exist … was not found`,
  looping `CheckSync → Syncing → Failure → Sleeping` permanently — unrecoverable without a database
  wipe. `calculate_substate_changes` now also rejects a non-monotonic version, so a same-version
  write fails loudly rather than silently corrupting the tree.
- `fix` — **`UP_ONLY` decides liveness by destruction, not by value presence.** A destroyed substate
  keeps its value until epoch GC prunes it, which runs `epoch_history_length` epochs back (default
  1), so anything destroyed inside that window was streamed as an up with its down filtered out and
  no later transition to correct it. An indexer syncing from scratch recorded those substates as live
  **permanently** — later rounds resume past that state version, so a UTXO spent shortly before the
  sync was reported unspent for the rest of that indexer's life.

### Execution Engine

- `feat!` — **The per-transaction WASM metering ceiling is raised from 100M to 250M points** (~12ms →
  ~30ms of validator CPU at the calibrated ~8.4M points/ms). The old ceiling put non-trivial
  cryptography out of reach of templates entirely and was 24x tighter than the native ceiling a
  single transaction already enjoys, for the same real CPU at the same price. Measured against a
  Groth16/BN254 verifier written entirely in WASM: at 100M nothing fitted except a single-input
  verification, at 96% of budget with nothing left for contract logic; at 250M a sixteen-input
  statement fits at 70% of budget and two verifies fit in one transaction. Sixty-four inputs
  deliberately does not fit — cost is linear at ~5.3M points each, so a statement that wide belongs
  behind a hash. The proposal budget is unchanged, so a block packs fewer heavy transactions rather
  than doing more work, and still admits at least 18 max-compute transactions
  (`MIN_MAX_COMPUTE_TRANSACTIONS_PER_BLOCK`). `FREE_COMPUTE_GRACE_POINTS` stays at 32M: nothing this
  expensive should be fundable before a fee is paid.
- `feat` — `TemplateTest::last_execution_points()` exposes what a call cost. `ExecuteResult` carries
  the points, but `call_function`/`call_method` return only the decoded value and drop the result.

### Indexer

- `feat` — **The substate cache is invalidated by the state transition stream instead of expiring on
  a timer**, so an unversioned read is servable indefinitely rather than for two seconds. Every read
  past that TTL cost a validator committee round trip — one per vault and per resource on a wallet
  account refresh. Presence in the cache is now validity, resting on three changes: the down feed is
  completed (`ALL_HASHES` was honoured on the up arm but not the down arm, so a subscriber never
  learned of a *terminal* down — a spent `Utxo` or `ConfidentialOutput`, or a `ValidatorFeePool`
  drained to zero — and would serve it as live forever); the cache moves from a `cacache` directory
  into the indexer's SQLite database so invalidation commits in the same transaction that advances
  `Key::SyncProgress`; and `ShardWatermarks` records per shard and **per process run** that a
  completion marker put the indexer level with the committee, so a fresh or restarted indexer serves
  nothing from cache until its first round lands. A cached head settles every version below it
  locally, answering a versioned read below the head without a round trip.
- `fix` — **The WASM module cache resolves against the configured data dir.** The indexer built its
  path from `config.indexer.data_dir` directly, which is relative by default and, unlike the
  validator node's, is never absolutised at load — so `wasm_cache` was created relative to the
  process working directory instead of `{base_path}/{network}/data/indexer/`.

### Swarm daemon

- `fix` — Log output cleaned up.

### Build & CI

- `ci` — **`consensus_tests` gets a 600s kill timeout**, up from the 240s `[profile.ci]` bound, scoped
  to that package alone. Runs were failing on `development` with 663 of 664 tests passing and one
  killed by the clock. These tests wait on wall-clock timers rather than on work finishing, so they
  are slow by construction and stretch further under runner contention — nineteen crossed the 60s
  slow mark within the same second, and several reported slow went on to pass at 73–93s.
- `test` — The walletd balance-change fixture builds its 205 bulk rows in one transaction instead of
  205, each of which was its own `with_write_tx` and so its own fsync. 4.3s → 0.6s locally, and the
  runtime no longer scales with fsync latency; it was timing out at 60s in CI while passing locally.
- `chore` — Dependency bumps: `taiki-e/install-action` 2.86.5 → 2.87.0, `browserslist` 4.28.6 →
  4.28.8 (swarm daemon web UI).

### Docs

- `docs` — Changelog entries added for 0.39.1, 0.39.2 and 0.39.3, which the changelog had skipped.

### Crate versions

`[workspace.package].version` moves to `0.40.0` (the whole tier-3 cohort). Independently versioned
crates affected:

| crate | version |
|---|---|
| `tari_ootle_wallet_crypto` | 0.41.0 → 0.42.0 |
| `tari_indexer_client` | 0.40.0 → 0.41.0 |
| `ootle-rs` | 0.21.0 → 0.22.0 |
| `tari_ootle_wallet_sdk` | 0.41.0 → 0.42.0 |
| `tari_ootle_wallet_storage_sqlite` | 0.41.0 → 0.42.0 |
| `tari_ootle_walletd_client` | 0.41.0 → 0.42.0 |

## [0.39.3](https://github.com/tari-project/tari-ootle/compare/v0.39.2...v0.39.3) (2026-08-26)

### ⚠️ Upgrade notes

- `feat!` — **Validators and indexers must be upgraded together.** The `sync_state` RPC now takes a list of
  `(shard, start_state_version)` cursors instead of a single shard, and its batch/completion messages
  carry the shard they belong to. The protocol name is unversioned, so a mixed-version pair cannot sync.
- `feat!` — **`tari_indexer_client`**: `TransactionEntry` gains a non-optional `source` field, so
  struct-literal construction breaks for downstream consumers. `ListRecentTransactionsRequest` gains an
  optional `source` filter.
- **Operator note** — indexer `transaction_retention_epochs` now defaults to `Some(50)` instead of retaining
  forever. Transaction rows written before the retention column existed carry epoch 0, so the first pruner
  pass after upgrading clears that backlog — whether or not gossip indexing is enabled.

### Indexer

- `feat` — **Transactions are indexed from network gossip**, not just from direct submissions. The indexer
  joins the transaction gossip topic as a full mesh participant: it validates what it receives, reports a
  verdict that propagates the transaction onward, and stores it. New config `index_gossiped_transactions`
  (default true) and `max_transaction_gossip_queue_bytes` (128 MB); `index_gossiped_transactions` is
  reported on `/info`, and metrics for received/accepted/rejected/ignored/stored/dropped plus queue depth
  are exposed behind the `metrics` feature.
  The stored transaction set is explicitly **best effort** — an indexer misses whatever was gossiped while
  it was offline or its queue was full, and nothing backfills it. Receipts for committed transactions stay
  complete from genesis; transaction bodies and never-committed transactions do not.
- `feat!` — `source` on `TransactionEntry` records where a transaction was learned of, with an optional
  `source` filter on the recent-transactions listing. A direct submission upgrades a row already stored
  from gossip.
- `feat` — **At most two state-sync streams per shard group** instead of one per shard — previously 257
  serialized round-trips per round at `P256`, roughly 21s at 80ms RTT against a 30s work interval.
  Per-shard progress is still recorded as completion markers arrive, so an interrupted stream resumes from
  where it got to. A truncated stream is now an error rather than silent success.
- `fix` — **Default substate rate limit raised** — wallets hit the limiter during a normal poll.

### Consensus

- `feat!` — **`sync_state` streams many shards over one stream.** Cursors must be non-empty, at most
  `num_preshards + 1`, in range, strictly ascending, and start above version 0; the responder streams each
  shard contiguously and closes it with its own marker. Verification granularity is unchanged — checkpoints
  already carry per-shard tree roots. The validator passes a single-element cursor list, so its sync
  behaviour is unchanged; ranged validator sync follows separately.

### Wallet

- `fix` — **Bindings**: `shortenString` returns short strings unchanged instead of producing overlapping
  output such as `Rick Ast...k Astley` in NFT metadata cards. Long addresses, hashes and Substate IDs keep
  their existing format.

### Swarm daemon

- `fix` — **Mining stops at the validator activation epoch.** Mining a fixed 20 blocks after registration
  crossed two epoch boundaries with 7 validators, so no committee ever ran consensus in the activation
  epoch and no checkpoint was ever written for it — every cold-starting validator then looped
  `Syncing → Failure → Sleeping` forever. The daemon now mines to a computed height and polls the validators
  for activation instead of mining further.
- `feat` — **Web UI redesigned around divergence.** A consensus spine shows validators as channels on a
  shared rail at the committee tip, so a lagging validator falls off it by a distance sized to its block
  deficit; a pool matrix replaces the per-node "transactions from other pools" tables. App shell with
  sidebar navigation, a live status bar and pages for validators, wallets, indexers, base layer and
  instances. Every feature of the old UI is kept.
- `fix` — One shared polling loop replaces per-card 1s timers; RPC failures surface as toasts; the log viewer
  gains level filters, search, follow and wrap; instance data deletion calls the method that actually
  exists (`delete_data`); reading `final_decision` no longer throws on unfinalized transactions;
  `npm run dev` proxies to the daemon.

### Build & CI

- `build` — **`tari_comms`, `tari_core` and `tari_p2p` are gone from the walletd, indexer and validator node
  dependency graphs.** `minotari_app_grpc` is now depended on with `default-features = false`; only its
  generated protobuf types were ever used. `tari_watcher`, `tari_swarm_daemon` and `integration_tests`
  still pull the wrapper grpc client crates, so a full `--workspace` build still unifies the feature on.
- `ci` — The `windows-arm64` binary build enters the `amd64_arm64` MSVC developer environment, so
  `liblmdb-sys`' bare `cl.exe` fallback resolves.

### Docs

- `docs` — New template-testing tutorial covering account setup, epoch and epoch-hash overrides, direct
  component-state inspection and rejected-transaction error assertions, backed by executable engine tests
  and a snippet-drift check.

### Crate versions

`[workspace.package].version` moves to `0.39.3` (the whole tier-3 cohort). Independently versioned crates
affected by the breaking `tari_indexer_client` change:

| crate | version |
|---|---|
| `tari_indexer_client` | 0.39.0 → 0.40.0 |
| `ootle-rs` | 0.20.0 → 0.21.0 |
| `tari_ootle_wallet_sdk` | 0.40.1 → 0.41.0 |
| `tari_ootle_wallet_storage_sqlite` | 0.40.0 → 0.41.0 |
| `tari_ootle_walletd_client` | 0.40.0 → 0.41.0 |

## [0.39.2](https://github.com/tari-project/tari-ootle/compare/v0.39.1...v0.39.2) (2026-08-24)

A hardening release: several Byzantine-reachable liveness bugs in consensus, a privilege escalation in
the wallet daemon, two engine metering/scoping gaps, and storage read paths that returned data from
branches or tables they should never have seen.

### ⚠️ Upgrade notes

- **A RocksDB migration (`v1`) runs on first start.** The substate-lock substate-id index now encodes its
  table prefix, so existing entries are rewritten. The migration is idempotent and safe to interrupt;
  a fresh database skips it. Timing is logged.

### Consensus

- `fix` — **Proposal votes are aggregated per voted block.** Votes were bucketed by `(epoch, height)` only,
  but a `ProposalVote`'s signature binds `(block_id, decision)` and its `block_height` is
  attacker-controlled. One Byzantine member voting for an invented `block_id` at the current height got
  its signature folded into the honest block's certificate, which every peer then rejected — repeatable
  every height, halting the chain well below the fault threshold. Safety was never at risk.
- `fix` — **The zero-block QC exemption requires genuine genesis shape.** It keyed off an all-zero header
  hash alone, so a peer could skip signature and quorum validation entirely with an arbitrary
  height/parent and push a receiving validator into `FallenBehind` catch-up. It now also requires a
  `ProposalCertificate` at height 0 with a zero parent; timeout certificates are never exempt.
- `fix` — **`MissingTransactionsRequest` is capped at 1000 transaction ids**, mirroring the bound the
  response path already applied. One small request could otherwise run an unbounded number of blocking
  store lookups inline on the consensus worker thread and echo back a correspondingly huge reply.

### State store

- `fix` — **Range query scans are bounded to their logical table.** Logical tables share a physical column
  family, separated by a leading prefix byte, and two of the four range methods left one end open.
  **Epoch GC therefore never made progress**: a single stored foreign proposal failed the scan, rolling
  back the whole cleanup transaction — block, QC and finalized-transaction pruning included — so the
  database grew without bound.
- `fix` — **Block diff queries are scoped to the queried branch.** The branch filter tested the query's own
  argument rather than the block that recorded the entry, so pending substate state from forked-out
  branches leaked into the evaluation of blocks on the surviving branch — the root cause of the flaky
  `catch_up_rewind_below_leaf_recovers` failure. Same-version changes now order `(version, is_down)`, so a
  DOWN supersedes its UP rather than the winner depending on block-id iteration order.
- `fix` — **The committed substate lock lookup returns the most recent lock**, not an arbitrary index match
  that could be superseded and then feed `try_lock`'s conflict decisions.
- `fix` — **The substate-lock substate-id index carries its table prefix** (migration `v1`, above). Latent
  until a new `SubstateId` variant reached the prefix range, at which point it would have silently
  overlapped another table.
- `fix` — `parked_block_remove_missing_transaction` uses the query-aware key iterator, so a decode error is
  propagated instead of being read as "still missing".

### Wallet

- `fix` — **`webrtc.start` no longer mints session tokens above the caller's own grant.** It parsed a
  caller-chosen permission set out of the request body and signed it verbatim — including
  `Permission::Admin`, which satisfies every check in the daemon. An integration holding only the
  deliberately least-powerful `webrtc` scope could escalate to a full Admin bearer token. Requested
  permissions are now filtered through what the caller was actually granted.

### Execution Engine

- `fix` — **`VaultAction::PayFee` bills and caps its stealth verification.** The canonical
  `stealth_transfer` path pre-charged bulletproof and balance-proof verification and enforced
  `max_fee_intent_transfers`; `PayFee` reached the same verification doing neither, so a template could
  loop `vault.pay_fee` against an unfunded vault and run full ZK verification on every validator in the
  committee, free and uncapped.
- `fix` — **Address allocations are scoped to the owning call frame.** They were tracked in one
  transaction-wide map keyed by small sequential integers and checked only for existence. A template a
  victim contract called could brute-force `GetAddress(0..k)`, learn the victim's expected future
  component address and create its own substate there first. Allocations now travel across frames the way
  buckets and proofs already do.
- `fix` — **A refused engine call fails the transaction.** The entrypoint can only answer WASM with a null
  pointer, and a template is free to ignore it, so refusals must be recorded out of band — three of the
  five null paths did not, and the transaction committed with the call's effect never applied. Version
  skew (a template calling an op an older engine cannot map) was the realistic route in.
- `fix` — The engine's own error log is no longer emitted through the metered `emit_log`, so the payer is
  not charged a `RuntimeCall` plus byte rate and a `max_logs` slot for the engine's diagnostic.
- `feat` — **Dispatcher decode panics are rendered from the template definition.** Templates emit a 5-byte
  marker and the engine expands it from the `FunctionDef` it is already invoking: `account` drops 6,691
  bytes (−2.4%). Non-breaking in both directions — an unmarked or unrenderable message passes through
  untouched.
- `perf` — **`template_lib` sheds the `EngineOp` `Debug` table and its last prose panics** (253-byte string
  table plus its match, eight `expect` messages, and a formatted null branch): another −651 bytes on
  `account`, −424 on `state`, −215 on `hello_world`.

### Indexer

- `feat` — **Optional retention window for submitted transactions.** `transaction_retention` (seconds, unset
  by default, so existing deployments are unchanged) and `transaction_prune_interval` (default 3600).
  Only the submitted transaction body and its locally recorded rejection reason are pruned — synced
  receipts are keyed independently and never touched. Pruning is by age alone, including still-pending
  transactions, since never-sequenced spam is the growth this targets. Deletes run in bounded batches so
  SQLite's database-wide write lock is not held over a large backlog.

### Docs & CI

- `docs` — New reference page on reducing template size; the stealth guide covers script-path spends
  (TIP-0006).
- `test` — Cucumber reports which node failed to start, and integration tests run on GitHub-hosted runners.
- `ci` — `cargo machete` runs on a hosted runner with a prebuilt binary instead of holding a self-hosted
  slot to compile a check that takes 0.6s.

## [0.39.1](https://github.com/tari-project/tari-ootle/compare/v0.39.0...v0.39.1) (2026-08-20)

### Release tooling

- `fix` — **`publish_crates.py` had `ootle_serde` in the wrong position**, after a crate that now depends on
  it, so a release run would fail partway through. It moves to just after `tari_bor`, and `check_order()`
  now cross-references the hand-maintained `CRATES` list against `cargo metadata` and aborts before
  anything is uploaded. `ootle_serde`'s versioned dev-dependency on `tari_template_lib` — a cycle that
  aborts packaging — is replaced by a local fixture.

## [0.39.0](https://github.com/tari-project/tari-ootle/compare/v0.38.0...v0.39.0) (2026-08-20)

### ⚠️ Upgrade notes

- `breaking` — **Testnet reset required.** Transaction ids, `TransactionReceipt`, `max_epoch`, the CBOR 128-bit
  encoding and the fee tables all changed — existing transactions are not compatible.
- `breaking` — **Templates must be rebuilt and republished against the latest `tari_template_lib`.** `Amount`'s
  wire format is now minicbor's native integer encoding (compact up to `u64::MAX`, bignum above)
  instead of a two-element digit array. It's smaller, but not backwards compatible — a template built
  on the old lib can't decode an `Amount` from the engine, or produce one it can read.
- `feat!` — **Manifests: the `info!`/`debug!`/`warn!`/`error!` macros are gone** (`Instruction::EmitLog` was
  removed). Logging *inside* a template is unchanged.
- `fix!` — **JS clients no longer patch `BigInt.prototype.toJSON`.** All bigints now serialize as strings,
  consistently — previously small values went out as numbers.

### Wallet 

- `feat!` — **`max_epoch` is now mandatory** on every transaction. `Transaction::builder(network, max_epoch)`
  takes it at construction; the network caps the window at 2160 epochs (~30 days). This change ensures 
	transactions have bounded validity.
- `feat!` — **Stealth transfers can pay up to 16 recipients in one statement** (was 8), which is cheaper than
  splitting across statements.
- `fix` — **Stealth fee estimates now settle before they're reported.** The estimate is priced from the
  transfer's actual shape locally, instead of dry-running at a guessed fee — no more estimates that
  describe a cheaper transaction than the one that gets built, and no extra network round trips.
- `fix` — **`final_fee` reports what was actually paid** (`total_fees_paid`), not the dry-run minimum. Partly
  paid transactions no longer look like they were overcharged.
- `fix` — **NFT transfers**: the wallet waits for local NFT records to update before answering, so a sent NFT
  disappears from your NFT list immediately; the web UI send dialog now stays on its result step
  instead of snapping back to the form.
- `feat` — **Web UI**: indexer liveness pill in the app bar (colour + tooltip with URL/epoch/error).
- `fix` — **Web UI**: the login screen no longer issues authenticated RPCs.
- `feat` — **ootle-wasm**: new `buildScriptPathWitness`, `buildStealthInputsStatementFromInputs` and
  `createTransferStatement` bindings — `PayTo::Conditions` outputs (hashlock/timelock/covenant MAST
  trees) can now be spent from the browser.

### Indexer

- `feat` — **Blob references are validated at ingress**, so malformed blob lists are rejected before signature
  verification instead of being forwarded to committees.
- `fix!` — `tari_indexer_client` / `@tari-project/indexer-client` pick up the bigint-as-string encoding.

### Consensus

- `feat!` — **Transactions have a bounded lifetime.** `max_transaction_validity_epochs = 2160` on every network;
  a transaction can no longer stay sequenceable forever.
- `feat!` — **Transaction ids exclude the seal signature's witness data**, so re-sealing an identical body can't
  produce N distinct valid transactions (no approve-once-execute-many).
- `feat!` — **`TransactionReceipt` gains a 32-byte intent commitment** — you can prove a transaction
  produced a given receipt without revealing the signers' or sealer's public keys.
- `fix!` — The exhaust burn rate is now bounded at build time, so a network can't be configured above
  the rate the fee estimate assumes.
- `chore!` — 128-bit CBOR integers use minicbor's native encoding (wire-breaking for values inside the
  CBOR integer range).

### Execution Engine

- `feat!` — **Publish fees retuned.** Free allowance 30 KiB → 96 KiB plus a flat 250,000 µT per publish. A large
  template (~260 KiB) drops from ~5 tTARI to ~2.7 tTARI; oversized templates stay expensive.
- `fix!` — **Receipts no longer carry `logs`**; use events for anything you need to index.
- `fix!` — **Receipts are now paid for.** The receipt substate is now charged as storage.
- `fix!` — **A log is charged for the bytes it carries**, not a flat per-call fee. Ordinary diagnostics
  cost about what they did; filling `max_logs` with 32 KiB entries no longer does.
- `fix!` — **Finalization fees are charged against the state actually persisted.** A transaction that commits
  only its fee intent is no longer priced against state that gets thrown away.
- `fix!` — **Compute is funded from the payment's unspent balance**, and the fee intent's compute is capped at a
  flat credit — free compute from repeated fee-intent aborts is closed.
- `fix!` — **Confidential accounting**: `total_supply` now tracks confidential commitments (previously
  only the revealed amount, so burnt value was reported forever), and the ElGamal value proof is sound
  (it was forgeable for any claimed value).
- `fix!` — **Engine calls are refused outside a template invocation** — from `tari_alloc`, `tari_free` or the
  response allocation. Nothing legitimate does this; it was a route to unmetered effects and, via
  `tari_alloc`, a node crash.

## [0.3.0](https://github.com/tari-project/tari-ootle/compare/v0.2.0...v0.3.0) (2023-12-19)

### ⚠ BREAKING CHANGES

* libp2p (#827)

### Features

* add version to template
  WASMs ([#835](https://github.com/tari-project/tari-ootle/issues/835)) ([8612eab](https://github.com/tari-project/tari-ootle/commit/8612eab9a1e6a713b04f86e624c5501fcf1c1808))
* do fee estimation in UI
  transfer ([#826](https://github.com/tari-project/tari-ootle/issues/826)) ([93bfd45](https://github.com/tari-project/tari-ootle/commit/93bfd452bd33fe8138d98df164bddbe7642ed650))
*
libp2p ([#827](https://github.com/tari-project/tari-ootle/issues/827)) ([9c29995](https://github.com/tari-project/tari-ootle/commit/9c29995cf0e3f5e7bbb875ea20e02dfa20eab540))
* **p2p:** peer-sync
  protocol ([#844](https://github.com/tari-project/tari-ootle/issues/844)) ([b49af42](https://github.com/tari-project/tari-ootle/commit/b49af421ec3cb72af6df42a952e26eeb4c286c03))
* request foreign
  blocks ([#760](https://github.com/tari-project/tari-ootle/issues/760)) ([7a59c4d](https://github.com/tari-project/tari-ootle/commit/7a59c4d4d2f3d3dcf55880e9a3fd12a5a73dc25e))
* show dummy blocks in
  ui ([#843](https://github.com/tari-project/tari-ootle/issues/843)) ([d5c77f6](https://github.com/tari-project/tari-ootle/commit/d5c77f6e2dbcaa9518343bc453df77c56924e219))

### Bug Fixes

* claim burn in the
  ui ([#841](https://github.com/tari-project/tari-ootle/issues/841)) ([ca80982](https://github.com/tari-project/tari-ootle/commit/ca80982672e4849f52ee5befca8e5e2e7106a003))
* cli argument
  duplicate ([#837](https://github.com/tari-project/tari-ootle/issues/837)) ([cb2d694](https://github.com/tari-project/tari-ootle/commit/cb2d694feb259683a0c58697b6d37d55c6a91867))
* force txs refetch on account change in
  UI ([#833](https://github.com/tari-project/tari-ootle/issues/833)) ([3e09ad5](https://github.com/tari-project/tari-ootle/commit/3e09ad5a2bb00dc4e309a9874f968cd17c34f7ed))
* **p2p/messaging:** single stream per
  connection ([#845](https://github.com/tari-project/tari-ootle/issues/845)) ([c0e09fe](https://github.com/tari-project/tari-ootle/commit/c0e09fefffaee7666c55c36025c039026109f21d))
* **swarm:** exit with error if unsupported seed
  multiaddr ([#836](https://github.com/tari-project/tari-ootle/issues/836)) ([b54bde8](https://github.com/tari-project/tari-ootle/commit/b54bde8178883a49038aa9b0ce6f57450e7184d6))

## [0.2.0](https://github.com/tari-project/tari-ootle/compare/v0.1.1...v0.2.0) (2023-12-08)

### ⚠ BREAKING CHANGES

* foreign broadcast reliability counter (#757)

### Features

* add transaction json download to
  ui ([#815](https://github.com/tari-project/tari-ootle/issues/815)) ([50c0ff5](https://github.com/tari-project/tari-ootle/commit/50c0ff5e5bacbcc2deb221b0cd55f42f61174551))
* disable buttons on send, add result
  dialog ([#813](https://github.com/tari-project/tari-ootle/issues/813)) ([1d146b8](https://github.com/tari-project/tari-ootle/commit/1d146b8190696b58dab6dbdae6abe8132319ea97))
* foreign broadcast reliability
  counter ([#757](https://github.com/tari-project/tari-ootle/issues/757)) ([f0dc999](https://github.com/tari-project/tari-ootle/commit/f0dc99954f634a8ac995a65bf06837edacede808))
* foreign proposal
  command ([#792](https://github.com/tari-project/tari-ootle/issues/792)) ([186b20d](https://github.com/tari-project/tari-ootle/commit/186b20d338cd3ee2c152037a6f4ba806148e44eb))
* **integration_tests:** new test for downed
  substates ([#798](https://github.com/tari-project/tari-ootle/issues/798)) ([5a0c47a](https://github.com/tari-project/tari-ootle/commit/5a0c47af80c5690869be218afdb1415742be4317))
* proper transaction signature and
  validation ([#791](https://github.com/tari-project/tari-ootle/issues/791)) ([e6a1082](https://github.com/tari-project/tari-ootle/commit/e6a108215c6e88a1e79738914aa89489836faf9f))
* set refresh balance interval to 5
  sec ([#819](https://github.com/tari-project/tari-ootle/issues/819)) ([61dfa4d](https://github.com/tari-project/tari-ootle/commit/61dfa4d996854910712b050970fdbc5c18496942))
* show substate version in dan wallet
  ui ([#810](https://github.com/tari-project/tari-ootle/issues/810)) ([89b2879](https://github.com/tari-project/tari-ootle/commit/89b287987109b26da70eed596185145d9f4afe24))
* sort TXs in UI, add
  timestamp ([#804](https://github.com/tari-project/tari-ootle/issues/804)) ([7dad32e](https://github.com/tari-project/tari-ootle/commit/7dad32ec1e8cac548b88d1d0bd4e4fe41d0db89a))

### Bug Fixes

* indexer settings in dan wallet
  ui ([#805](https://github.com/tari-project/tari-ootle/issues/805)) ([068d1ad](https://github.com/tari-project/tari-ootle/commit/068d1ad1a3cd4b9eb1a378694dc9714febca1b85))
*
propagation ([#799](https://github.com/tari-project/tari-ootle/issues/799)) ([ef10627](https://github.com/tari-project/tari-ootle/commit/ef10627ea77af78d9c4799dd115b164f2507e942))
* shard range
  computation ([#796](https://github.com/tari-project/tari-ootle/issues/796)) ([892fe0c](https://github.com/tari-project/tari-ootle/commit/892fe0ce871e6c1a8a9f70d9c51ec196f86cd175))
* shorten string on small
  strings ([#823](https://github.com/tari-project/tari-ootle/issues/823)) ([064c540](https://github.com/tari-project/tari-ootle/commit/064c54067ce09b798022bda2e0bdcbbe7a31bb8e))
* **wallet_daemon_web_ui:** send correct max_fee param on
  transfers ([#795](https://github.com/tari-project/tari-ootle/issues/795)) ([0f07b81](https://github.com/tari-project/tari-ootle/commit/0f07b8161ce6493d76d549fc2fd1b8dd9d38dfd2))
