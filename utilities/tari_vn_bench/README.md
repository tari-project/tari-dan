# tari-vn-bench

Measures whether a machine can keep up with validator-node consensus, and reports the spec that
implies.

The bar it grades against is not "the node starts" but "the node keeps voting". A validator that
cannot execute and vote inside `pacemaker_block_time` misses proposals; missing
`missed_proposal_suspend_threshold` of them suspends it, `missed_proposal_evict_threshold` evicts
it. Every threshold in the report is derived from the consensus constants for the network being
sized for, so re-tuning a constant re-tunes the verdict with it.

## Running it

The binary is self-contained. The templates it executes are compiled into `tari_template_builtin`,
so the machine under test needs no toolchain, no checkout and no network.

```bash
cargo build --release -p tari_vn_bench
scp target/release/tari-vn-bench candidate:/tmp/

# On the candidate, with the machine idle:
/tmp/tari-vn-bench --label mainnet-candidate-1 --data-dir /var/lib/tari
```

`--data-dir` must be on the volume the node's data directory will live on. A fast root disk says
nothing about a slow attached volume.

The binary links only `libc` and `libgcc_s`, but it inherits the build host's glibc floor. Check it
before copying:

```bash
objdump -T target/release/tari-vn-bench | grep -o 'GLIBC_[0-9.]*' | sort -V -u | tail -1
ssh candidate 'ldd --version | head -1'
```

If the candidate's glibc is older, build in a container matching its distribution rather than
statically linking — the engine's WASM compiler is not worth fighting musl over.

Useful flags:

| Flag | Effect |
| --- | --- |
| `--json` | Report as JSON on stdout; progress stays on stderr, so it redirects cleanly |
| `--compare <file.json>` | Diff the headline metrics against a previous run |
| `--vn-pid <pid>` | Read a running node's peak RSS and check it against the derived memory ceiling |
| `--network <name>` | Constants to grade against (default `mainnet`) |
| `--quick` | Fewer samples; wider margins than it appears to report |
| `--skip-execution`, `--skip-native`, `--skip-storage` | Run a subset |

Run it on an idle machine. Every figure is a minimum-of-N, so competing load can only make the
machine look worse — but a report taken under load says nothing about the machine's ceiling. A noisy
run is flagged rather than silently graded.

**Build it with `--release`.** A debug build runs the engine several times slower than the binary a
validator actually runs. The tool detects this and fails the report rather than publish the numbers.

## What it measures, and why that

### Execution — the binding constraint

A block's commands execute serially, so cores do not help; single-thread speed does. Two budgets
bound a block independently and both are measured:

- **weight** (`max_block_validation_weight`) — a size/IO estimate, enforced on receive. Measured by filling a block
  with canonical transfers and executing it, exactly as a replica does. The resulting weight/s is the same figure a
  running node logs from `on_propose`, so a report can be checked against production logs.
- **execution points** (`max_block_validation_execution_points`) — the actual metered cost, also enforced on receive.
  Measured as a marginal WASM rate via a two-point slope, which cancels the fixed per-transaction overhead and yields
  the quantity the budgets are denominated in.

Transactions are built before the clock starts: building signs them, and signing is the wallet's
cost. State changes are not committed, so every execution starts from the same funded state and the
sample is repeatable.

**The execution figures are an upper bound.** They measure execution against an in-memory state
store; a node's propose and vote loops also read and write substates, maintain the state tree and
commit to RocksDB. The node logs its own observed weight/s from `on_propose` — once the node is
running, that log is the ground truth and this tool is the pre-flight estimate. Expect the node's
figure to be lower, and size the margin accordingly.

### Native verification — the unmetered half

- **Signature verification** is what mempool admission pays for every gossiped transaction, including ones the node
  never sequences. It is the one cost an attacker can raise for free.
- **Stealth transfer verification** runs outside the WASM meter and is charged at a network-wide points price fitted
  by wall-clock equivalence against the WASM rate. The report compares measured time against what that price implies
  *on this host*: where native crypto is disproportionately slow, the execution-point budget understates what a
  stealth-heavy block costs, and the points headroom is optimistic.

### Storage

- **fsync latency** is the floor under every block commit — RocksDB syncs its write-ahead log — and the page cache
  cannot fake it. The tail matters more than the median, because epoch GC and state sync issue syncs in bursts.
- **Sequential write throughput** governs state sync and compaction: how long joining a shard group or recovering
  takes.

Random-read IOPS is deliberately absent. Without `O_DIRECT` or the ability to drop the page cache —
neither of which a benchmark may assume it can do — any figure would be the cache's, not the
device's. Use `fio` if the fsync result is marginal.

### Encoded sizes and network requirements — measured once, not per host

Bandwidth and disk are products of a rate the protocol fixes and a size the implementation fixes.
The rate is knowable from the consensus constants; the size is not, because a block command carries
`Evidence`, and evidence holds a full `SubstateId` for every input and output in every shard group
the transaction touches. That cannot be estimated to better than a factor of two by reading the
types, so the tool builds the real structures and encodes them with the real codec.

Nothing in that phase touches the host — no network, no database, not even an engine — so the
figures are identical on every machine. They are reported but deliberately **not graded**: grading
this box against a number every validator shares would say nothing about this box.

Both requirements are split the same way:

- a **floor** the protocol fixes, computable today, which does not move with adoption. Publish this one.
- a **rate** per unit of traffic, so an adoption assumption can be multiplied through rather than baked into a figure
  that goes stale.

The floor matters for the same reason the CPU floor did: a validator must survive the worst its
peers can send, not the average. A link sized for today's traffic drops proposals the first time a
committee saturates.

Sizes are CBOR, which is exactly what the state store persists, so the disk figures are direct.
Consensus messages go over the wire as protobuf, which packs the same data slightly tighter, so
treat the bandwidth figures as a modest over-estimate rather than a floor.

What is **not** covered: growth of live substate state, the genuinely unbounded term. Bounding it
needs bytes-per-committed-substate measured against a running network with representative traffic.
Both requirements scale inversely with the block interval, and that interval is not a constant.
`pacemaker_block_time` is the liveness ceiling for a *quiet* network; under load the next block is
proposed as soon as the previous quorum certificate forms, so the rate is set by execution plus one
round of vote collection. The report therefore projects two scenarios rather than one, and anchors
the saturated interval to this host's measured execution time — which makes it an upper bound on
demand, since that measurement excludes storage work.

`--epoch-minutes` (default 20, matching 10 layer-one blocks at a 2 minute L1 block time),
`--epoch-history-length` and `--committee-rtt-ms` parameterise the projection.

### Memory — derived, then corroborated

Memory is the one axis a benchmark cannot measure by running: the ceiling is what the node
allocates when every bounded buffer is simultaneously full, and a quiet machine never reaches it.
So the report derives it from the caps the code enforces, and separates:

- **capped** terms, with a hard limit in the code — a flood cannot exceed them;
- **estimated** terms, bounded only by library defaults or by traffic — where a memory surprise comes from.

What is left in the estimated column is bounded by message counts rather than byte budgets
(gossipsub's caches and per-connection send queues) or scales with a block's contents (the execution
working set), so both depend on figures nothing caps — message size and peer count.

The table here is for a node running stock configuration. A node logs its own version of it at
startup, and checks it against `MemAvailable`, so a differently configured node reports its own
ceiling rather than this one.

`--vn-pid` reads a running node's `VmHWM` and checks it against the derived ceiling. A node sitting
far below is normal: the capped terms are burst-and-attack ceilings, not steady state.

**Linux only.** Host detection, the memory readings and `--vn-pid` all read `/proc`. The tool builds
and runs elsewhere, but reports no CPU model, core count or memory, which makes most of the verdict
meaningless — run it on the machine you intend to validate from.

## Reading the verdict

`PASS` / `WARN` / `FAIL` per axis, worst axis wins. The grades are coarse because the decision is
binary — provision this machine or don't — and the numbers behind each grade are printed alongside
it for anyone who wants to argue with the thresholds.
