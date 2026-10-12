#!/usr/bin/env python3
#   Copyright 2026 The Tari Project
#   SPDX-License-Identifier: BSD-3-Clause
"""Swarm smoke test: exercises the main transaction paths against a running local swarm.

Stages (default order; select with --only / --skip):
  flood         🚰 Faucet-claim flood via transaction_generator + transaction_submitter
  templates     📦 Publish freshly built templates through the wallet daemon
  stealth       🥷 tTARI stealth transfers between two wallet accounts
  token         🪙 Mint a stealth token from a new template, stealth-transfer it, dry-run vs real fee
  component     🧮 Create a component from the new template and check its state across transactions
  swap          🔄 Create a tTARI/token liquidity pool from the builtin template, contribute and swap
  spend_back    ↩️  The recipient spends the stealth outputs it received
  concurrent    🧵 Concurrent stealth transfers from one account
  nfts          🖼️  Mint testnet NFTs from the builtin faucet
  negative      🚫 Low fee, overspend, unauthorised withdraw and orphaned resource must all fail
  multi_wallet  👥 Stealth transfer to a second wallet daemon (skipped unless one is running)
  claim_burn    🔥 Burn tTARI on the base layer and wait for the wallet daemon to auto-claim it
  epoch         ⏳ Mine into the next epoch and keep flooding until consensus crosses it (opt-in: --epoch)
  fees          💸 Fee report, compared with the previous run's fees
  health        🩺 Validators are running, agree on the epoch, and advanced during the run

Every wallet transaction is also looked up on the indexer: its result must be a committed accept and
the substates it wrote must be served by the indexer.

Ports are discovered from the swarm daemon (default http://localhost:8080) unless --wallet-url /
--indexer-url are given. The wallet daemon must use `None` auth.

    scripts/smoke_test.py
    scripts/smoke_test.py --only templates,token,swap
    scripts/smoke_test.py --flood 500 --templates 5 --epoch
"""

import argparse
import base64
import json
import os
import re
import secrets
import statistics
import subprocess
import sys
import tempfile
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
TEMPLATE_DIR = REPO / "scripts" / "smoke_test_template"
TEMPLATE_WASM = TEMPLATE_DIR / "target" / "wasm32-unknown-unknown" / "release" / "smoke_test_template.wasm"
FEES_FILE = REPO / "data" / "smoke_test" / "fees.json"
TARI_TOKEN = "resource_" + "01" * 32
LIQUIDITY_POOL_TEMPLATE = "template_" + "00" * 31 + "02"
TARI = 1_000_000
SMOKE_ACCOUNT = "smoketest"
RECV_ACCOUNT = "smoketest-recv"
FEE_DRIFT_WARN = 0.20
HEIGHT_SPREAD_WARN = 5
# A stealth transfer's fee is not refunded, so this is what each one costs.
STEALTH_MAX_FEE = 20_000
# The recipient accumulates many small outputs across runs, and each input adds verification cost.
SPEND_BACK_MAX_FEE = 100_000

STAGES = [
    "flood", "templates", "stealth", "token", "component", "swap", "spend_back", "concurrent",
    "nfts", "negative", "multi_wallet", "claim_burn", "epoch", "fees", "health",
]
OPT_IN_STAGES = {"epoch"}
STAGE_TITLES = {
    "flood": "🚰 Faucet-claim flood",
    "templates": "📦 Publish templates",
    "stealth": "🥷 Stealth transfers",
    "token": "🪙 Stealth token from new template",
    "component": "🧮 Component state",
    "swap": "🔄 Liquidity pool swap",
    "spend_back": "↩️  Recipient spends received outputs",
    "concurrent": "🧵 Concurrent stealth transfers",
    "nfts": "🖼️  Mint faucet NFTs",
    "negative": "🚫 Transactions that must fail",
    "multi_wallet": "👥 Transfer to a second wallet",
    "claim_burn": "🔥 Claim a base-layer burn",
    "epoch": "⏳ Flood across an epoch boundary",
    "fees": "💸 Fee report",
    "health": "🩺 Validator health",
}

TEMPLATE_ADDR_RE = re.compile(r"^template_[0-9a-f]{64}$")
RESOURCE_ADDR_RE = re.compile(r"^resource_[0-9a-f]{64}$")
COMPONENT_ADDR_RE = re.compile(r"^component_[0-9a-f]{64}$")

USE_COLOR = sys.stdout.isatty() and not os.environ.get("NO_COLOR")


def _c(code, s):
    return f"\033[{code}m{s}\033[0m" if USE_COLOR else s


def bold(s):
    return _c("1", s)


def dim(s):
    return _c("2", s)


def green(s):
    return _c("32", s)


def red(s):
    return _c("31", s)


def yellow(s):
    return _c("33", s)


PRINT_LOCK = threading.Lock()


def info(msg):
    with PRINT_LOCK:
        print(f"   {msg}", flush=True)


def detail(msg):
    with PRINT_LOCK:
        print(dim(f"      {msg}"), flush=True)


def warn(msg):
    with PRINT_LOCK:
        print(yellow(f"      ⚠️  {msg}"), flush=True)


def short(addr):
    if isinstance(addr, str) and "_" in addr and len(addr) > 30:
        prefix, hexpart = addr.rsplit("_", 1)
        return f"{prefix}_{hexpart[:8]}…{hexpart[-6:]}"
    if isinstance(addr, str) and len(addr) == 64:
        return f"{addr[:8]}…{addr[-6:]}"
    return str(addr)


def fmt_tari(micro):
    return f"{int(micro) / TARI:,.6f}".rstrip("0").rstrip(".") + " tTARI"


class StageFailed(Exception):
    pass


class StageSkipped(Exception):
    pass


# --------------------------------------------------------------------------------------------- RPC


class JsonRpc:
    def __init__(self, url, timeout=120):
        self.url = url
        self.timeout = timeout
        self.token = None
        self._id = 0

    def call(self, method, params=None, timeout=None):
        self._id += 1
        body = json.dumps({"jsonrpc": "2.0", "id": self._id, "method": method, "params": params or {}}).encode()
        headers = {"Content-Type": "application/json"}
        if self.token:
            headers["Authorization"] = f"Bearer {self.token}"
        req = urllib.request.Request(self.url, data=body, headers=headers)
        try:
            with urllib.request.urlopen(req, timeout=timeout or self.timeout) as resp:
                payload = json.load(resp)
        except urllib.error.URLError as e:
            raise StageFailed(f"{method}: cannot reach {self.url}: {e}") from e
        if payload.get("error"):
            err = payload["error"]
            data = f" {json.dumps(err['data'])}" if err.get("data") else ""
            raise StageFailed(f"{method}: {err.get('message')}{data}")
        return payload["result"]


class Wallet(JsonRpc):
    def login(self):
        self.token = None
        self.token = self.call("auth.request", {"permissions": ["Admin"], "credentials": "None"})["token"]
        self.login_at = time.monotonic()

    def call(self, method, params=None, timeout=None):
        # JWTs expire after 5 minutes; a long run would otherwise outlive the token.
        if self.token and method != "auth.request" and time.monotonic() - self.login_at > 240:
            self.login()
        return super().call(method, params, timeout)

    def wait(self, tx_id, timeout_secs=120):
        res = self.call(
            "transactions.wait_result",
            {"transaction_id": tx_id, "timeout_secs": timeout_secs},
            timeout=timeout_secs + 30,
        )
        if res.get("timed_out"):
            raise StageFailed(f"transaction {short(tx_id)} timed out after {timeout_secs}s")
        status = res.get("status")
        if status != "Accepted":
            raise StageFailed(f"transaction {short(tx_id)} finished as {status}: {reject_reason(res.get('result'))}")
        return res


def http_get(url, timeout=30):
    try:
        with urllib.request.urlopen(url, timeout=timeout) as resp:
            return resp.status, json.load(resp)
    except urllib.error.HTTPError as e:
        return e.code, None
    except urllib.error.URLError as e:
        raise StageFailed(f"cannot reach {url}: {e}") from e


def reject_reason(finalize):
    if not finalize:
        return "no result"
    fee_only = find_key(finalize, "AcceptFeeRejectRest")
    found = (fee_only[1] if isinstance(fee_only, list) and len(fee_only) > 1 else None) or find_key(finalize, "Reject")
    found = found or find_key(finalize, "message") or find_key(finalize, "reason")
    if isinstance(found, dict) and len(found) == 1:
        (kind, msg), = found.items()
        found = f"{kind}: {msg}" if isinstance(msg, str) else found
    if isinstance(found, str):
        return found
    return json.dumps(found)[:400] if found else json.dumps(finalize)[:400]


def find_key(obj, key):
    if isinstance(obj, dict):
        if key in obj:
            return obj[key]
        for v in obj.values():
            r = find_key(v, key)
            if r is not None:
                return r
    elif isinstance(obj, list):
        for v in obj:
            r = find_key(v, key)
            if r is not None:
                return r
    return None


def find_strings(obj, pattern):
    out = []
    if isinstance(obj, str):
        if pattern.match(obj):
            out.append(obj)
    elif isinstance(obj, dict):
        for k, v in obj.items():
            out += find_strings(k, pattern) + find_strings(v, pattern)
    elif isinstance(obj, list):
        for v in obj:
            out += find_strings(v, pattern)
    return list(dict.fromkeys(out))


class Swarm:
    """What the swarm daemon reports is running. Empty when the daemon is unreachable."""

    def __init__(self, url):
        self.instances = []
        try:
            self.instances = JsonRpc(f"{url.rstrip('/')}/json_rpc", timeout=10).call("list_instances")["instances"]
        except StageFailed:
            pass

    def running(self, instance_type):
        return [i for i in self.instances if i.get("is_running") and i["instance_type"] == instance_type]

    def wallet_urls(self):
        return [f"http://127.0.0.1:{i['ports']['jrpc']}/json_rpc" for i in self.running("TariWalletDaemon")]

    def indexer_url(self):
        idx = self.running("TariIndexer")
        return f"http://127.0.0.1:{idx[0]['ports']['api']}" if idx else None

    def instance_id_for(self, wallet_url):
        for i in self.running("TariWalletDaemon"):
            if f":{i['ports']['jrpc']}/" in wallet_url:
                return i["id"]
        return None

    def validators(self):
        return [(i["name"], f"http://127.0.0.1:{i['ports']['jrpc']}/json_rpc") for i in self.running("TariValidatorNode")]


# ------------------------------------------------------------------------------------------ helpers


def run(cmd, cwd=REPO, env=None):
    proc = subprocess.run(cmd, cwd=cwd, env={**os.environ, **(env or {})}, text=True, capture_output=True)
    if proc.returncode != 0:
        tail = "\n".join(((proc.stdout or "") + (proc.stderr or "")).strip().splitlines()[-15:])
        raise StageFailed(f"`{' '.join(map(str, cmd))}` exited {proc.returncode}\n{tail}")
    return proc.stdout or ""


def build_template(nonce):
    run(
        ["cargo", "build", "--release", "--target", "wasm32-unknown-unknown"],
        cwd=TEMPLATE_DIR,
        env={"SMOKE_TEST_NONCE": nonce},
    )
    return TEMPLATE_WASM.read_bytes()


def balances(wallet, account):
    res = wallet.call("accounts.get_balances", {"account": {"Name": account}, "refresh": True})
    return {b["resource_address"]: b for b in res["balances"]}


def revealed(entry):
    return int(entry["balance"]) if entry else 0


def total(entry):
    return int(entry["balance"]) + int(entry["confidential_balance"]) if entry else 0


def fee_main(amount=TARI):
    return f"""fn fee_main() {{
    let account = var!["account"];
    account.pay_fee({amount});
}}"""


# --------------------------------------------------------------------------------------------- stages


class SmokeTest:
    def __init__(self, args):
        self.args = args
        self.swarm = Swarm(args.swarm_url)
        self.wallet = None
        self.indexer_url = None
        self.smoke = None  # create_or_get response for the smoketest account
        self.recv = None  # create_or_get response for the account receiving stealth transfers
        self.template = None
        self.token = None  # (resource_address, symbol)
        self.fees = {}  # operation -> [required fees, µT]
        self.overcharges = {}  # operation -> [fee paid above the required fee, µT]
        self.health_before = None

    # ---- setup

    def setup(self):
        self.wallet_url = self.args.wallet_url or next(iter(self.swarm.wallet_urls()), None)
        self.indexer_url = (self.args.indexer_url or self.swarm.indexer_url() or "").rstrip("/")
        if not self.wallet_url or not self.indexer_url:
            raise StageFailed("no running wallet daemon / indexer found; pass --wallet-url / --indexer-url")
        self.wallet = Wallet(self.wallet_url)
        self.wallet.login()
        info(f"👛 wallet   {dim(self.wallet_url)}")
        info(f"🔎 indexer  {dim(self.indexer_url)}")

        self.smoke = self.wallet.call("accounts.create_or_get", {"account": {"Name": SMOKE_ACCOUNT}})
        verb = "created" if self.smoke["created"] else "found"
        info(f"🧪 account  {bold(SMOKE_ACCOUNT)} {verb} {dim(short(self.smoke['account']['component_address']))}")
        self.recv = self.wallet.call("accounts.create_or_get", {"account": {"Name": RECV_ACCOUNT}})
        info(f"📥 account  {bold(RECV_ACCOUNT)} {dim(short(self.recv['account']['component_address']))}")
        self.ensure_funds()

        if self.swarm.validators():
            self.health_before = self.validator_status()
            heights = ", ".join(str(s["height"]) for s in self.health_before.values())
            info(f"🩺 {len(self.health_before)} validators · heights {heights}")

    def ensure_funds(self):
        bal = revealed(balances(self.wallet, SMOKE_ACCOUNT).get(TARI_TOKEN))
        if bal < self.args.min_balance * TARI:
            info(f"🚰 balance {fmt_tari(bal)} is low, claiming from the faucet…")
            res = self.wallet.call(
                "accounts.create_free_test_coins",
                {"account": {"Name": SMOKE_ACCOUNT}, "max_fee": 100_000},
                timeout=180,
            )
            self.indexer_check(res["transaction_id"], "faucet_claim")
            detail(f"+{fmt_tari(res['amount'])}")
            bal = revealed(balances(self.wallet, SMOKE_ACCOUNT).get(TARI_TOKEN))
        info(f"💰 balance  {fmt_tari(bal)}")

    # ---- transaction plumbing

    def indexer_check(self, tx_id, op):
        """The indexer must report the transaction committed and serve the substates it wrote."""
        deadline = time.monotonic() + 60
        while True:
            status, body = http_get(f"{self.indexer_url}/transactions/{tx_id}/result")
            finalized = (body or {}).get("result", {}).get("Finalized") if status == 200 else None
            if finalized:
                break
            if time.monotonic() > deadline:
                raise StageFailed(f"indexer has no finalized result for {short(tx_id)} after 60s (HTTP {status})")
            time.sleep(1)
        if finalized.get("final_decision") != "Commit":
            raise StageFailed(f"indexer reports {short(tx_id)} as {finalized.get('final_decision')}")
        finalize = finalized["execution_result"]["finalize"]
        accept = finalize.get("result", {}).get("Accept")
        if accept is None:
            raise StageFailed(f"indexer reports {short(tx_id)} not accepted: {reject_reason(finalize)}")
        for substate_id, *_ in accept.get("up_substates", [])[:3]:
            code, _ = http_get(f"{self.indexer_url}/substates/{substate_id}")
            if code != 200:
                raise StageFailed(f"indexer cannot serve {short(substate_id)} written by {short(tx_id)} (HTTP {code})")
        # Fees paid from stealth inputs are not refunded, so the overcharge is reported separately and
        # the required fee is what the fee report tracks.
        paid = int(finalize["total_fees_required"])
        self.fees.setdefault(op, []).append(paid)
        overcharge = int(finalize["fee_receipt"].get("total_fee_overcharge") or 0)
        if overcharge:
            self.overcharges.setdefault(op, []).append(overcharge)
        return paid, finalize

    def wait(self, tx_id, op):
        result = self.wallet.wait(tx_id)
        paid, finalize = self.indexer_check(tx_id, op)
        return result, paid, finalize

    def submit_manifest(self, manifest, variables, op, signer=None, max_fee=TARI, dry_run=False):
        signer = signer or self.smoke
        res = self.wallet.call("transactions.submit_manifest", {
            "manifest": manifest,
            "variables": variables,
            "seal_signer_key_id": signer["account"]["owner_key_id"],
            "signing_key_ids": [],
            "max_fee": max_fee,
            "dry_run": dry_run,
            "blobs": {},
        })
        if dry_run:
            return res
        return self.wait(res["transaction_id"], op)

    def stealth_transfer(self, resource, amount, dest, label, sender=SMOKE_ACCOUNT, wallet=None,
                         selection="PreferRevealed", max_fee=STEALTH_MAX_FEE):
        res = (wallet or self.wallet).call("accounts.stealth_transfer", {
            "owner_account": {"Name": sender},
            "fee_params": {"input_selection": selection, "pay_fee_with_swap": None},
            "input_selection": selection,
            "resource_address": resource,
            "transfers": [{
                "destination_address": dest,
                "blinded_output_amount": amount,
                "revealed_output_amount": 0,
                "pay_to": "StealthPublicKey",
                "attach_sender_address": True,
            }],
            "max_fee": max_fee,
            "dry_run": False,
        })
        _, paid, _ = self.wait(res["transaction_id"], "stealth_transfer")
        detail(f"🥷 {label} → {short(dest)} · fee {paid} µT")

    def ensure_template(self):
        if not self.template:
            info("📦 no template from this run yet, publishing one…")
            self.publish_templates(1)

    def ensure_token(self):
        if not self.token:
            info("🪙 no token from this run yet, minting one…")
            self.stage_token()

    # ---- flood

    def build_tx_tools(self):
        if not self.args.no_build and not getattr(self, "_built_tools", False):
            info("🔨 building transaction_generator + transaction_submitter…")
            run(["cargo", "build", "--release", "-p", "transaction_generator", "-p", "transaction_submitter"])
            self._built_tools = True

    def flood(self, n, quiet=False):
        bins = REPO / "target" / "release"
        with tempfile.TemporaryDirectory(prefix="smoke-") as tmp:
            out = Path(tmp) / "claims.bin"
            if not quiet:
                info(f"✍️  generating {bold(n)} faucet-claim transactions…")
            run([bins / "transaction_generator", "write", "-n", str(n), "-o", out, "--indexer-url", self.indexer_url])
            if not quiet:
                info(f"🚀 submitting to {dim(self.indexer_url)}…")
            started = time.monotonic()
            output = run([bins / "transaction_submitter", "stress-test", "-f", out, "-a", self.indexer_url, "-y"])
            elapsed = time.monotonic() - started

        stats = {}
        for key, label in [
            ("submitted", "Transactions submitted"),
            ("committed", "Fully committed"),
            ("fee_only", "Fee charged, execution rejected"),
            ("rejected", "Rejected"),
            ("errored", "Errored"),
        ]:
            m = re.search(rf"{re.escape(label)}:\s*(\d+)", output)
            stats[key] = int(m.group(1)) if m else None
        if stats["committed"] is None:
            raise StageFailed("could not parse submitter summary:\n" + "\n".join(output.splitlines()[-15:]))
        breakdown = (
            f"fee-only {stats['fee_only']} · rejected {stats['rejected']} · errored {stats['errored']}"
        )
        if stats["committed"] != n:
            reasons = sorted({l.strip() for l in output.splitlines() if re.search(r"reject|error|fail|abort", l, re.I)
                              and not re.match(r"\s*(Rejected|Errored|Fee charged)", l)})
            raise StageFailed(f"only {stats['committed']}/{n} claims committed ({breakdown})"
                              + "".join(f"\n      {r[:200]}" for r in reasons[:5]))
        if quiet:
            return elapsed
        detail(
            f"submitted {stats['submitted']} · committed {stats['committed']} · fee-only {stats['fee_only']} · "
            f"rejected {stats['rejected']} · errored {stats['errored']} · {elapsed:.1f}s "
            f"(~{stats['committed'] / max(elapsed, 0.001):.1f} TPS)"
        )
        return elapsed

    def stage_flood(self):
        self.build_tx_tools()
        elapsed = self.flood(self.args.flood)
        return f"{self.args.flood} claims committed in {elapsed:.1f}s"

    def consensus_epoch(self):
        """The lowest epoch any validator's consensus has reached. Consensus adopts a new epoch some
        time after the base layer mines into it, so the epoch manager's view is not used."""
        validators = self.swarm.validators()
        if not validators:
            raise StageSkipped("swarm daemon not reachable; cannot read the validators' consensus epoch")
        return min(JsonRpc(url, timeout=10).call("get_consensus_status")["epoch"] for _, url in validators)

    def stage_epoch(self):
        self.build_tx_tools()
        start_epoch = self.consensus_epoch()
        if self.args.epoch_mine:
            swarm = JsonRpc(f"{self.args.swarm_url.rstrip('/')}/json_rpc", timeout=120)
            swarm.call("mine", {"num_blocks": self.args.epoch_mine})
            info(f"⛏️  mined {self.args.epoch_mine} base-layer blocks")
        info(f"⏳ flooding until consensus moves past epoch {bold(start_epoch)} (timeout {self.args.epoch_timeout}s)")
        deadline = time.monotonic() + self.args.epoch_timeout
        batches = 0
        while True:
            elapsed = self.flood(self.args.flood, quiet=True)
            batches += 1
            epoch = self.consensus_epoch()
            detail(f"batch {batches}: {self.args.flood} committed in {elapsed:.1f}s · consensus epoch {epoch}")
            if epoch > start_epoch:
                break
            if time.monotonic() > deadline:
                raise StageFailed(f"consensus epoch still {epoch} after {self.args.epoch_timeout}s")
        # One more batch entirely inside the new epoch.
        self.flood(self.args.flood, quiet=True)
        return f"consensus epoch {start_epoch} → {epoch}, {batches + 1} batches of {self.args.flood} all committed"

    # ---- templates

    def publish_templates(self, count):
        for i in range(count):
            binary = build_template(secrets.token_hex(8))
            res = self.wallet.call("transactions.publish_template", {
                "binary": base64.b64encode(binary).decode(),
                "fee_account": {"Name": SMOKE_ACCOUNT},
                "max_fee": 2 * TARI,
                "detect_inputs": True,
                "dry_run": False,
            })
            result, paid, _ = self.wait(res["transaction_id"], "publish_template")
            addrs = find_strings(result.get("result"), TEMPLATE_ADDR_RE)
            if not addrs:
                raise StageFailed(f"publish {short(res['transaction_id'])} accepted but its diff has no template")
            code, _ = http_get(f"{self.indexer_url}/templates/{addrs[0].removeprefix('template_')}")
            if code != 200:
                raise StageFailed(f"indexer cannot serve the definition of {short(addrs[0])} (HTTP {code})")
            self.template = addrs[0]
            detail(f"📦 {i + 1}/{count} {short(addrs[0])} · {len(binary) // 1024} KiB · fee {paid} µT")

    def stage_templates(self):
        self.publish_templates(self.args.templates)
        return f"{self.args.templates} template(s) published and served by the indexer"

    # ---- stealth

    def stage_stealth(self):
        dest = self.recv["address"]
        before = total(balances(self.wallet, RECV_ACCOUNT).get(TARI_TOKEN))
        amounts = [TARI, 2 * TARI, 3 * TARI][: self.args.stealth]
        for a in amounts:
            self.stealth_transfer(TARI_TOKEN, a, dest, fmt_tari(a))
        after = total(balances(self.wallet, RECV_ACCOUNT).get(TARI_TOKEN))
        if after - before != sum(amounts):
            raise StageFailed(f"recipient balance moved by {fmt_tari(after - before)}, expected {fmt_tari(sum(amounts))}")
        return f"{len(amounts)} stealth transfer(s), recipient +{fmt_tari(sum(amounts))}"

    # ---- token

    def stage_token(self):
        self.ensure_template()
        symbol = "SMK" + secrets.token_hex(2).upper()
        supply = 1_000_000
        manifest = f"""
use {self.template} as SmokeToken;
{fee_main()}
fn main() {{
    let account = var!["account"];
    let coins = SmokeToken::mint("{symbol}", {supply});
    account.deposit(coins);
}}
"""
        variables = {"account": self.smoke["account"]["component_address"]}
        dry = self.submit_manifest(manifest, variables, "token_mint", dry_run=True)
        required = int(dry.get("required_fees") or 0)
        _, paid, _ = self.submit_manifest(manifest, variables, "token_mint")
        detail(f"🪙 minted {supply:,} {bold(symbol)} via {short(self.template)} · fee {paid} µT (dry run {required} µT)")
        if paid > required:
            raise StageFailed(f"real fee {paid} µT exceeds the dry run's required fee {required} µT")

        bals = balances(self.wallet, SMOKE_ACCOUNT)
        entry = next((b for b in bals.values() if b.get("token_symbol") == symbol), None)
        if entry is None or total(entry) != supply:
            raise StageFailed(f"{SMOKE_ACCOUNT} does not hold {supply} {symbol} (saw {entry})")
        self.token = (entry["resource_address"], symbol)
        detail(f"💼 {SMOKE_ACCOUNT} holds {total(entry):,} {symbol} {dim(short(self.token[0]))}")

        self.stealth_transfer(self.token[0], 1000, self.recv["address"], f"1,000 {symbol}")
        recv = total(balances(self.wallet, RECV_ACCOUNT).get(self.token[0]))
        if recv != 1000:
            raise StageFailed(f"recipient holds {recv} {symbol}, expected 1000")
        return f"{symbol} minted, deposited and stealth-transferred; fee within dry run"

    # ---- component

    def stage_component(self):
        self.ensure_template()
        variables = {"account": self.smoke["account"]["component_address"]}
        result, _, _ = self.submit_manifest(f"""
use {self.template} as SmokeToken;
{fee_main()}
fn main() {{
    let counter = SmokeToken::new();
}}
""", variables, "component_create")
        component = next(iter(find_strings(result.get("result"), COMPONENT_ADDR_RE)), None)
        components = [c for c in find_strings(result.get("result"), COMPONENT_ADDR_RE)
                      if c != variables["account"]]
        if not components:
            raise StageFailed("component creation accepted but its diff has no new component")
        component = components[0]
        detail(f"🧮 created {short(component)}")
        variables["counter"] = component
        rounds = 3
        for i in range(rounds):
            self.submit_manifest(f"""
{fee_main()}
fn main() {{
    let counter = var!["counter"];
    counter.increment();
}}
""", variables, "component_call")
        detail(f"➕ incremented {rounds} times in {rounds} transactions")
        self.submit_manifest(f"""
{fee_main()}
fn main() {{
    let counter = var!["counter"];
    counter.assert_count({rounds}u64);
}}
""", variables, "component_call")
        detail(f"🔍 assert_count({rounds}) committed")
        return f"component state persisted across {rounds + 2} transactions"

    # ---- swap

    def stage_swap(self):
        self.ensure_token()
        token, symbol = self.token
        account = self.smoke["account"]["component_address"]
        variables = {"account": account, "tari": TARI_TOKEN, "token": token}
        # OwnerRule::OwnedBySigner and AccessRule::AllowAll, CBOR-encoded as [variant, fields].
        result, paid, _ = self.submit_manifest(f"""
use {LIQUIDITY_POOL_TEMPLATE} as Pool;
{fee_main()}
fn main() {{
    let account = var!["account"];
    let pool = Pool::create(cbor!([0, []]), cbor!([0, []]), var!["tari"], var!["token"], metadata!({{"name": "smoke"}}), None);
    let tari_in = account.withdraw(var!["tari"], {10 * TARI});
    let token_in = account.withdraw(var!["token"], 100000);
    let (lp, change_a, change_b) = pool.contribute(tari_in, token_in);
    account.deposit(lp);
    account.deposit(change_a);
    account.deposit(change_b);
}}
""", variables, "pool_create")
        pools = [c for c in find_strings(result.get("result"), COMPONENT_ADDR_RE) if c != account]
        if not pools:
            raise StageFailed("pool creation accepted but its diff has no new component")
        variables["pool"] = pools[0]
        detail(f"🏊 pool {short(pools[0])} seeded with 10 tTARI + 100,000 {symbol} · fee {paid} µT")

        before = total(balances(self.wallet, SMOKE_ACCOUNT).get(token))
        _, paid, _ = self.submit_manifest(f"""
{fee_main()}
fn main() {{
    let account = var!["account"];
    let pool = var!["pool"];
    let input = account.withdraw(var!["tari"], {TARI});
    let output = pool.swap(input);
    account.deposit(output);
}}
""", variables, "pool_swap")
        gained = total(balances(self.wallet, SMOKE_ACCOUNT).get(token)) - before
        # Constant product: 100,000 * 1 / (10 + 1) ≈ 9,090.
        detail(f"🔄 swapped 1 tTARI → {gained:,} {symbol} · fee {paid} µT")
        if not 9000 <= gained <= 9100:
            raise StageFailed(f"swap returned {gained} {symbol}, expected ~9,090")
        return f"pool created, contributed to and swapped 1 tTARI → {gained:,} {symbol}"

    # ---- spend back

    def stage_spend_back(self):
        recv_bal = balances(self.wallet, RECV_ACCOUNT)
        if total(recv_bal.get(TARI_TOKEN)) < 2 * TARI:
            info("🥷 recipient has too little tTARI, sending some first…")
            self.stealth_transfer(TARI_TOKEN, 3 * TARI, self.recv["address"], fmt_tari(3 * TARI))
        before = total(balances(self.wallet, SMOKE_ACCOUNT).get(TARI_TOKEN))
        self.stealth_transfer(TARI_TOKEN, TARI // 2, self.smoke["address"], fmt_tari(TARI // 2),
                              sender=RECV_ACCOUNT, selection="PreferConfidential", max_fee=SPEND_BACK_MAX_FEE)
        after = total(balances(self.wallet, SMOKE_ACCOUNT).get(TARI_TOKEN))
        if after - before != TARI // 2:
            raise StageFailed(f"{SMOKE_ACCOUNT} moved by {fmt_tari(after - before)}, expected 0.5 tTARI")
        done = ["0.5 tTARI"]
        if self.token and total(recv_bal.get(self.token[0])) >= 500:
            token, symbol = self.token
            before = total(balances(self.wallet, SMOKE_ACCOUNT).get(token))
            self.stealth_transfer(token, 500, self.smoke["address"], f"500 {symbol}",
                                  sender=RECV_ACCOUNT, selection="PreferConfidential", max_fee=SPEND_BACK_MAX_FEE)
            gained = total(balances(self.wallet, SMOKE_ACCOUNT).get(token)) - before
            if gained != 500:
                raise StageFailed(f"{SMOKE_ACCOUNT} gained {gained} {symbol}, expected 500")
            done.append(f"500 {symbol}")
        return f"recipient spent received outputs: {' + '.join(done)}"

    # ---- concurrent

    def stage_concurrent(self):
        n = self.args.concurrent
        dest = self.recv["address"]
        before = total(balances(self.wallet, RECV_ACCOUNT).get(TARI_TOKEN))
        errors = []

        def send(i):
            try:
                self.stealth_transfer(TARI_TOKEN, TARI, dest, f"#{i + 1} {fmt_tari(TARI)}")
            except StageFailed as e:
                errors.append(f"#{i + 1}: {e}")

        threads = [threading.Thread(target=send, args=(i,)) for i in range(n)]
        for t in threads:
            t.start()
        for t in threads:
            t.join()
        if errors:
            raise StageFailed(f"{len(errors)}/{n} concurrent transfers failed: {errors[0]}")
        after = total(balances(self.wallet, RECV_ACCOUNT).get(TARI_TOKEN))
        if after - before != n * TARI:
            raise StageFailed(f"recipient moved by {fmt_tari(after - before)}, expected {fmt_tari(n * TARI)}")
        return f"{n} concurrent stealth transfers committed, recipient +{fmt_tari(n * TARI)}"

    # ---- nfts

    def stage_nfts(self):
        n = self.args.nfts
        res = self.wallet.call("nfts.mint_faucet_nft", {
            "account": {"Name": SMOKE_ACCOUNT},
            "mutable_data": {"name": f"Smoke test {time.strftime('%H:%M:%S')}"},
            "number_to_mint": n,
            "max_fee": 200_000,
        }, timeout=180)
        paid, _ = self.indexer_check(res["transaction_id"], "nft_mint")
        detail(f"🖼️  minted {n} NFT(s) · fee {paid} µT · tx {short(res['transaction_id'])}")
        return f"{n} faucet NFT(s) minted"

    # ---- negative

    def expect_failure(self, label, manifest, variables, must_contain=None, signer=None, max_fee=TARI):
        try:
            self.submit_manifest(manifest, variables, "negative", signer=signer, max_fee=max_fee)
        except StageFailed as e:
            msg = str(e)
            if must_contain and must_contain.lower() not in msg.lower():
                raise StageFailed(f"{label}: failed for an unexpected reason: {msg}") from e
            detail(f"🚫 {label}: {msg.split(': ', 1)[-1][:150]}")
            return
        raise StageFailed(f"{label}: transaction was accepted")

    def ensure_on_chain(self, name, account):
        """The account's component address, after claiming faucet coins into the account if the network does not
        hold its component yet. Receiving stealth transfers leaves an account without one."""
        address = account["account"]["component_address"]
        code, _ = http_get(f"{self.indexer_url}/substates/{address}")
        if code == 404:
            info(f"🚰 {bold(name)} has no component on the network yet, claiming from the faucet…")
            res = self.wallet.call(
                "accounts.create_free_test_coins",
                {"account": {"Name": name}, "max_fee": 100_000},
                timeout=180,
            )
            self.indexer_check(res["transaction_id"], "faucet_claim")
        elif code != 200:
            raise StageFailed(f"indexer cannot serve {short(address)} (HTTP {code})")
        return address

    def stage_negative(self):
        account = self.smoke["account"]["component_address"]
        variables = {"account": account}

        self.expect_failure("fee too low", f"""
{fee_main(1)}
fn main() {{
}}
""", variables, max_fee=1)

        bal = revealed(balances(self.wallet, SMOKE_ACCOUNT).get(TARI_TOKEN))
        self.expect_failure("overspend", f"""
{fee_main()}
fn main() {{
    let account = var!["account"];
    let coins = account.withdraw(var!["tari"], {bal + TARI});
    account.deposit(coins);
}}
""", {**variables, "tari": TARI_TOKEN})

        victim = self.ensure_on_chain(RECV_ACCOUNT, self.recv)
        self.expect_failure("withdraw from another key's account", f"""
{fee_main()}
fn main() {{
    let account = var!["account"];
    let victim = var!["victim"];
    let coins = victim.withdraw(var!["tari"], 1);
    account.deposit(coins);
}}
""", {**variables, "victim": victim, "tari": TARI_TOKEN},
            must_contain="denied")

        self.ensure_template()
        self.expect_failure("orphaned resource", f"""
use {self.template} as SmokeToken;
{fee_main()}
fn main() {{
    SmokeToken::create_orphan("ORPHAN");
}}
""", variables, must_contain="orphaned substate")
        return "all 4 invalid transactions failed"

    # ---- multi wallet

    def stage_multi_wallet(self):
        others = [u for u in self.swarm.wallet_urls() if u != self.wallet_url]
        if self.args.peer_wallet_url:
            others = [self.args.peer_wallet_url]
        if not others:
            raise StageSkipped("no second wallet daemon running (start one in the swarm or pass --peer-wallet-url)")
        peer = Wallet(others[0])
        peer.login()
        peer_acc = peer.call("accounts.create_or_get", {"account": {"Name": "smoketest-peer"}})
        info(f"👥 peer {dim(others[0])} account {short(peer_acc['account']['component_address'])}")
        before = total(balances(peer, "smoketest-peer").get(TARI_TOKEN))
        self.stealth_transfer(TARI_TOKEN, TARI, peer_acc["address"], fmt_tari(TARI))
        deadline = time.monotonic() + 60
        while True:
            after = total(balances(peer, "smoketest-peer").get(TARI_TOKEN))
            if after - before == TARI:
                break
            if time.monotonic() > deadline:
                raise StageFailed(f"peer wallet balance moved by {fmt_tari(after - before)} after 60s, expected 1 tTARI")
            time.sleep(2)
        return "peer wallet detected the incoming stealth output"

    # ---- claim burn

    def stage_claim_burn(self):
        """The swarm's console wallet writes the burn proof to the shared burn_proofs directory, which the
        wallet daemon watches and claims from once consensus is past the epoch the burn was mined in."""
        instance_id = self.swarm.instance_id_for(self.wallet_url)
        if instance_id is None:
            raise StageSkipped("wallet daemon is not managed by the swarm; cannot burn into it")
        if not self.swarm.running("MinoTariConsoleWallet"):
            raise StageSkipped("no console wallet running in the swarm")
        swarm = JsonRpc(f"{self.args.swarm_url.rstrip('/')}/json_rpc", timeout=300)
        amount = self.args.burn * TARI
        account = self.smoke["account"]["component_address"]
        res = swarm.call("burn_funds", {"amount": amount, "wallet_instance_id": instance_id, "account_name": SMOKE_ACCOUNT})
        info(f"🔥 burned {fmt_tari(amount)} to {bold(SMOKE_ACCOUNT)} at consensus epoch {self.consensus_epoch()}")

        # The claim tombstones the burn's commitment, which identifies the claim among other wallet traffic.
        proof_url = f"{self.args.swarm_url.rstrip('/')}{res['url']}"
        tombstone = None
        claim_tx = None
        deadline = time.monotonic() + self.args.burn_timeout
        next_mine = 0
        while claim_tx is None:
            if time.monotonic() > deadline:
                waiting = "for the burn proof" if tombstone is None else f"for a claim writing {short(tombstone)}"
                raise StageFailed(f"still waiting {waiting} after {self.args.burn_timeout}s "
                                  f"(consensus epoch {self.consensus_epoch()})")
            # The proof needs base-layer confirmations, and the claim needs consensus past the burn's epoch.
            if time.monotonic() >= next_mine:
                swarm.call("mine", {"num_blocks": 10})
                detail(f"⛏️  mined 10 blocks · consensus epoch {self.consensus_epoch()}")
                next_mine = time.monotonic() + 30
            if tombstone is None:
                status, proof = http_get(proof_url)
                if status == 200:
                    commitment = proof["claim_proof"]["output_proof"]["output"]["commitment"]
                    tombstone = f"tombstone_{commitment}"
                    detail(f"📜 burn proof mined in L1 epoch {proof['mined_in_epoch']}")
            else:
                txs = self.wallet.call("transactions.list", {"status": "Accepted", "account": account})["transactions"]
                claim_tx = next((t["id"] for t in txs if tombstone in json.dumps(t["finalize"]["result"])), None)
            time.sleep(3)
        paid, _ = self.indexer_check(claim_tx, "claim_burn")
        return f"{fmt_tari(amount)} burn claimed in {short(claim_tx)} · fee {paid} µT"

    # ---- fees

    def stage_fees(self):
        previous = {}
        try:
            previous = json.loads(FEES_FILE.read_text())
        except (OSError, ValueError):
            pass
        current = {op: int(statistics.median(v)) for op, v in self.fees.items() if op != "negative"}
        if not current:
            raise StageSkipped("no fees recorded this run")
        drifted = []
        for op in sorted(current):
            fee = current[op]
            prev = previous.get(op)
            if prev:
                change = (fee - prev) / prev
                delta = f"{change:+.1%} vs {prev} µT"
                if abs(change) > FEE_DRIFT_WARN:
                    drifted.append(op)
                    delta = yellow(delta)
                else:
                    delta = dim(delta)
            else:
                delta = dim("new")
            over = self.overcharges.get(op)
            over = dim(f"  (+{int(statistics.median(over)):,} µT overcharged)") if over else ""
            detail(f"{op:<18} {fee:>9,} µT  {delta}{over}")
        FEES_FILE.parent.mkdir(parents=True, exist_ok=True)
        FEES_FILE.write_text(json.dumps({**previous, **current}, indent=2, sort_keys=True))
        if drifted:
            warn(f"fees moved more than {FEE_DRIFT_WARN:.0%}: {', '.join(drifted)}")
            return f"{len(current)} operations; {len(drifted)} drifted ({', '.join(drifted)})"
        return f"{len(current)} operations, none drifted more than {FEE_DRIFT_WARN:.0%}"

    # ---- health

    def validator_status(self):
        status = {}
        for name, url in self.swarm.validators():
            rpc = JsonRpc(url, timeout=10)
            cs = rpc.call("get_consensus_status")
            em = rpc.call("get_epoch_manager_stats")
            group = em.get("committee_info", {}).get("shard_group") or {}
            status[name] = {
                "state": cs["state"],
                "epoch": cs["epoch"],
                "height": cs["height"],
                "group": (group.get("start"), group.get("end_inclusive")),
            }
        return status

    def stage_health(self):
        if not self.swarm.validators():
            raise StageSkipped("swarm daemon not reachable; cannot list validators")
        now = self.validator_status()
        problems = []
        for name, s in now.items():
            if s["state"] != "Running":
                problems.append(f"{name} is {s['state']}")
            prev = (self.health_before or {}).get(name)
            if prev and prev["epoch"] != s["epoch"]:
                moved = f"epoch {prev['epoch']} height {prev['height']} → epoch {s['epoch']} height {s['height']}"
            elif prev:
                moved = f"epoch {s['epoch']}  height {prev['height']} → {s['height']}"
            else:
                moved = f"epoch {s['epoch']}  height {s['height']}"
            detail(f"{name:<20} {s['state']:<9} {moved}")
            # Heights restart at each epoch change.
            if prev and (s["epoch"], s["height"]) <= (prev["epoch"], prev["height"]):
                problems.append(f"{name} did not advance (epoch {s['epoch']} height {s['height']})")
        epochs = {s["epoch"] for s in now.values()}
        if len(epochs) > 1:
            problems.append(f"validators disagree on the epoch: {sorted(epochs)}")
        groups = {}
        for s in now.values():
            groups.setdefault(s["group"], []).append(s["height"])
        for group, heights in groups.items():
            if max(heights) - min(heights) > HEIGHT_SPREAD_WARN:
                problems.append(f"shard group {group} heights spread {min(heights)}..{max(heights)}")
        if problems:
            raise StageFailed("; ".join(problems))
        return f"{len(now)} validators running in epoch {epochs.pop()} and advancing"


# ----------------------------------------------------------------------------------------------- main


def main():
    p = argparse.ArgumentParser(
        description="Ootle swarm smoke test",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog=__doc__,
    )
    p.add_argument("--swarm-url", default="http://localhost:8080")
    p.add_argument("--wallet-url", help="walletd JSON-RPC URL, e.g. http://127.0.0.1:5100/json_rpc")
    p.add_argument("--peer-wallet-url", help="second walletd for the multi_wallet stage")
    p.add_argument("--indexer-url", help="indexer REST URL, e.g. http://127.0.0.1:12500")
    p.add_argument("--flood", type=int, default=200, help="faucet claims per flood (default 200)")
    p.add_argument("--templates", type=int, default=3, help="templates to publish (default 3)")
    p.add_argument("--stealth", type=int, default=3, choices=[1, 2, 3], help="tTARI stealth transfers (default 3)")
    p.add_argument("--concurrent", type=int, default=3, help="concurrent stealth transfers (default 3)")
    p.add_argument("--nfts", type=int, default=3, help="faucet NFTs to mint (default 3)")
    p.add_argument("--min-balance", type=int, default=100, help="claim faucet coins below this many tTARI")
    p.add_argument("--burn", type=int, default=10, help="tTARI to burn in the claim_burn stage (default 10)")
    p.add_argument("--burn-timeout", type=int, default=600, help="seconds to wait for a burn to be claimed")
    p.add_argument("--epoch", action="store_true", help="also run the epoch stage")
    p.add_argument("--epoch-timeout", type=int, default=1800, help="seconds to wait for an epoch change")
    p.add_argument("--epoch-mine", type=int, default=12,
                   help="base-layer blocks the swarm mines at the start of the epoch stage (0 to not mine)")
    p.add_argument("--only", help=f"comma-separated subset of: {','.join(STAGES)}")
    p.add_argument("--skip", help="comma-separated stages to skip")
    p.add_argument("--no-build", action="store_true", help="use existing target/release generator/submitter")
    p.add_argument("--fail-fast", action="store_true", help="stop at the first failing stage")
    args = p.parse_args()

    all_stages = STAGES
    if args.only:
        selected = args.only.split(",")
    else:
        selected = [s for s in all_stages if s not in OPT_IN_STAGES or (s == "epoch" and args.epoch)]
    skipped = set(args.skip.split(",")) if args.skip else set()
    for s in selected + list(skipped):
        if s not in all_stages:
            p.error(f"unknown stage {s!r}; choose from {', '.join(all_stages)}")
    selected = [s for s in all_stages if s in selected and s not in skipped]

    print(bold("\n🔥 Ootle swarm smoke test\n"))
    test = SmokeTest(args)
    started = time.monotonic()
    try:
        print(bold("⚙️  Setup"))
        test.setup()
    except StageFailed as e:
        print(red(f"   ❌ setup failed: {e}"))
        return 1

    results = []
    for i, stage in enumerate(selected, 1):
        print(bold(f"\n[{i}/{len(selected)}] {STAGE_TITLES[stage]}"))
        t0 = time.monotonic()
        try:
            summary = getattr(test, f"stage_{stage}")()
            dt = time.monotonic() - t0
            print(green(f"   ✅ {summary}") + dim(f"  ({dt:.1f}s)"))
            results.append((stage, "pass", summary, dt))
        except StageSkipped as e:
            dt = time.monotonic() - t0
            print(yellow(f"   ⏭️  {e}"))
            results.append((stage, "skip", str(e), dt))
        except StageFailed as e:
            dt = time.monotonic() - t0
            print(red(f"   ❌ {e}") + dim(f"  ({dt:.1f}s)"))
            results.append((stage, "fail", str(e).splitlines()[0], dt))
            if args.fail_fast:
                break
        except KeyboardInterrupt:
            print(yellow("\n   ⏹️  interrupted"))
            results.append((stage, "fail", "interrupted", time.monotonic() - t0))
            break

    marks = {"pass": "✅", "fail": "❌", "skip": "⏭️ "}
    print(bold("\n📋 Summary"))
    for stage, outcome, summary, dt in results:
        text = red(summary) if outcome == "fail" else (yellow(summary) if outcome == "skip" else summary)
        print(f"   {marks[outcome]} {STAGE_TITLES[stage]:<38} {dim(f'{dt:6.1f}s')}  {text}")
    for stage in selected[len(results):]:
        print(f"   ⏹️  {STAGE_TITLES[stage]:<38} {dim('   not run')}")

    failed = sum(o == "fail" for _, o, _, _ in results) + len(selected) - len(results)
    passed = sum(o == "pass" for _, o, _, _ in results)
    skipped_n = sum(o == "skip" for _, o, _, _ in results)
    total_s = time.monotonic() - started
    skipped_note = f", {skipped_n} skipped" if skipped_n else ""
    if failed == 0:
        print(green(bold(f"\n🎉 {passed} stages passed{skipped_note} in {total_s:.1f}s\n")))
        return 0
    print(red(bold(f"\n💥 {failed}/{len(selected)} stages failed{skipped_note} ({total_s:.1f}s)\n")))
    return 1


if __name__ == "__main__":
    sys.exit(main())
