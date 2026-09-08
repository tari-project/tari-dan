#!/usr/bin/env python3
"""
Publish Tari Ootle crates to crates.io in dependency order.

Prerequisites:
    cargo login  # authenticate with crates.io

Usage:
    ./scripts/publish_crates.py                                     # show what would be published
    ./scripts/publish_crates.py --dry-run                           # test builds without publishing
    ./scripts/publish_crates.py --execute                           # publish all
    ./scripts/publish_crates.py -p tari_engine --execute            # single crate
    ./scripts/publish_crates.py --from tari_engine_types --execute  # resume
"""

import argparse
import json
import subprocess
import sys
import time
import urllib.request
import urllib.error

# Version tiers (concern domains). Tier 3 is the only workspace-versioned cohort;
# every other tier is independently versioned (each crate carries its own version):
#   1 = Stable/foundational — shared primitives, rarely change
#   2 = Template — template authoring crates & the built-in templates
#   3 = Core — workspace-versioned (shares [workspace.package].version), moves together
#   4 = Wallet — wallet SDK, clients & storage, decoupled from the core version
#
# Topological publish order — dependencies before dependents.
# (crate_name, crate_directory, tier)
CRATES = [
    ("tari_bor", "crates/tari_bor", 1),
    ("ootle_serde", "crates/ootle_serde", 1),
    ("ootle-network", "crates/ootle_network", 1),
    ("tari_template_abi", "crates/template_abi", 2),
    ("tari_template_lib_types", "crates/template_lib_types", 2),
    ("ootle_byte_type", "crates/ootle_byte_type", 1),
    ("tari_template_macros", "crates/template_macros", 2),
    ("tari_template_lib", "crates/template_lib", 2),
    ("tari_ootle_template_metadata", "crates/template_metadata", 2),
    ("tari_ootle_template_build", "crates/template_build", 2),
    ("tari_engine_types", "crates/engine_types", 3),
    ("tari_ootle_common_types", "crates/common_types", 3),
    ("tari_ootle_wallet_crypto", "crates/wallet/crypto", 4),
    ("tari_ootle_address", "crates/ootle_address", 1),
    ("tari_ootle_transaction", "crates/transaction", 3),
    # Tier 2, not tier 3: its only build dependency is tari_template_lib_types, so it has no build-time coupling to
    # the core cohort (its tari_engine_types / tari_ootle_transaction deps are dev-only) and carries its own version.
    ("tari_template_builtin", "crates/template_builtin", 2),
    ("tari_transaction_manifest", "crates/transaction_manifest", 3),
    ("tari_engine", "crates/engine", 3),
    ("tari_consensus_types", "crates/consensus_types", 3),
    ("tari_indexer_client", "clients/tari_indexer_client", 4),
    ("ootle-wasm-core", "crates/ootle_wasm/core", 3),
    ("ootle-wasm", "crates/ootle_wasm/wasm", 3),
    ("tari_template_test_tooling", "crates/template_test_tooling", 3),
    ("ootle_ledger_common", "crates/wallet/ledger/common", 4),
    ("ootle_ledger_client", "crates/wallet/ledger/client", 4),
    ("ootle-rs", "crates/wallet/ootle-rs", 4),
    ("tari_ootle_wallet_sdk", "crates/wallet/sdk", 4),
    ("tari_ootle_wallet_storage_sqlite", "crates/wallet/storage_sqlite", 4),
    ("tari_ootle_walletd_client", "clients/wallet_daemon_client", 4),
]

TIER_LABELS = {1: "stable", 2: "template", 3: "core", 4: "wallet"}

WAIT_SECS = 20

# Colors
RED = "\033[0;31m"
GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
NC = "\033[0m"


def get_local_version(crate_name: str, crate_dir: str) -> str:
    """Get the local version of a crate from cargo metadata."""
    result = subprocess.run(
        [
            "cargo", "metadata", "--no-deps", "--format-version", "1",
            "--manifest-path", f"{crate_dir}/Cargo.toml",
        ],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo metadata failed for {crate_name}: {result.stderr}")

    metadata = json.loads(result.stdout)
    for pkg in metadata["packages"]:
        if pkg["name"] == crate_name:
            return pkg["version"]
    raise RuntimeError(f"Package {crate_name} not found in cargo metadata")


def check_order() -> list:
    """Return (dependent, dependency) pairs whose normal dep is published too late.

    CRATES is hand-maintained, so a newly added dependency edge can silently
    invalidate the order. Only normal (and build) deps constrain it. Dev-dep
    cycles are tolerated, but only when the dev-dependency is declared path-only
    (no version), as tari_engine does for tari_template_test_tooling — cargo
    resolves versioned dev-deps against the index when packaging, so a versioned
    one in a cycle fails the publish.
    """
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        capture_output=True, text=True,
    )
    if result.returncode != 0:
        raise RuntimeError(f"cargo metadata failed: {result.stderr}")

    position = {name: i for i, (name, _, _) in enumerate(CRATES)}
    violations = []
    for pkg in json.loads(result.stdout)["packages"]:
        if pkg["name"] not in position:
            continue
        for dep in pkg["dependencies"]:
            if dep["name"] not in position or (dep.get("kind") or "normal") == "dev":
                continue
            if position[dep["name"]] > position[pkg["name"]]:
                violations.append((pkg["name"], dep["name"]))
    return sorted(violations)


def crate_index_path(name: str) -> str:
    """Convert crate name to sparse registry index path."""
    n = len(name)
    if n <= 2:
        return f"{n}/{name}"
    elif n == 3:
        return f"3/{name[0]}/{name}"
    else:
        return f"{name[:2]}/{name[2:4]}/{name}"


def published_versions(crate_name: str):
    """Every version of a crate on crates.io, as [(version, yanked), ...].

    Returns [] for a crate that has never been published, and None when the
    lookup itself failed — callers must distinguish the two, since "no versions"
    and "we could not find out" lead to opposite advice.

    Reads the sparse index rather than the JSON API: one unauthenticated request
    against the same source cargo resolves from.
    """
    url = f"https://index.crates.io/{crate_index_path(crate_name)}"
    try:
        req = urllib.request.Request(url)
        with urllib.request.urlopen(req, timeout=10) as resp:
            body = resp.read().decode()
    except urllib.error.HTTPError as e:
        return [] if e.code == 404 else None
    except (urllib.error.URLError, TimeoutError):
        return None

    versions = []
    for line in body.splitlines():
        try:
            entry = json.loads(line)
        except json.JSONDecodeError:
            continue
        if entry.get("vers"):
            versions.append((entry["vers"], bool(entry.get("yanked"))))
    return versions


def is_published(crate_name: str, version: str) -> bool:
    """Check if a specific version of a crate exists on crates.io.

    A failed lookup answers False so a publish is attempted rather than skipped;
    cargo rejects a duplicate version itself.
    """
    return any(v == version for v, _ in published_versions(crate_name) or ())


def cargo_publish(crate_name: str, dry_run: bool = False) -> bool:
    """Publish a crate using cargo publish."""
    cmd = ["cargo", "publish", "-p", crate_name, "--no-verify", "--allow-dirty"]
    if dry_run:
        cmd.append("--dry-run")
    result = subprocess.run(cmd)
    return result.returncode == 0


def main():
    parser = argparse.ArgumentParser(
        description="Publish Tari Ootle crates to crates.io in dependency order.",
    )
    mode_group = parser.add_mutually_exclusive_group()
    mode_group.add_argument(
        "--execute", action="store_true",
        help="Actually publish. Without this flag, everything is a dry-run.",
    )
    mode_group.add_argument(
        "--dry-run", action="store_true", dest="dry_run",
        help="Test builds with 'cargo publish --dry-run' without actually publishing.",
    )
    parser.add_argument(
        "-p", "--package", action="append", default=[],
        help="Only publish specific package(s). Can be repeated.",
    )
    parser.add_argument(
        "--from", dest="from_crate",
        help="Resume publishing from the named crate (skips earlier ones).",
    )
    parser.add_argument(
        "--wait", type=int, default=WAIT_SECS,
        help=f"Seconds to wait between publishes (default: {WAIT_SECS}).",
    )
    parser.add_argument(
        "--list", action="store_true",
        help="Print the publish order and exit.",
    )
    args = parser.parse_args()

    violations = check_order()
    if violations:
        print(f"{RED}Error: publish order in CRATES is stale — "
              f"these crates are listed before a dependency:{NC}")
        for dependent, dep in violations:
            print(f"  {dependent} needs {dep} published first")
        sys.exit(1)

    # --list mode
    if args.list:
        print("Publish order:")
        for name, crate_dir, tier in CRATES:
            ver = get_local_version(name, crate_dir)
            label = TIER_LABELS[tier]
            print(f"  {name} ({ver}) [{label}] — {crate_dir}")
        return

    # Build the list of crates to process
    crates_to_publish = []
    if args.from_crate:
        found = False
        for name, crate_dir, tier in CRATES:
            if name == args.from_crate:
                found = True
            if found:
                crates_to_publish.append((name, crate_dir, tier))
        if not found:
            print(f"{RED}Error: crate '{args.from_crate}' not found in publish order{NC}")
            sys.exit(1)
    elif args.package:
        pkg_set = set(args.package)
        for name, crate_dir, tier in CRATES:
            if name in pkg_set:
                crates_to_publish.append((name, crate_dir, tier))
        unknown = pkg_set - {name for name, _, _ in CRATES}
        if unknown:
            print(f"{RED}Error: unknown crate(s): {', '.join(unknown)}{NC}")
            sys.exit(1)
    else:
        crates_to_publish = list(CRATES)

    if args.dry_run:
        print(f"{YELLOW}=== DRY RUN (testing builds with cargo publish --dry-run) ==={NC}")
    elif not args.execute:
        print(f"{YELLOW}=== DRY RUN (pass --execute to publish for real) ==={NC}")
    print()

    published_crates = []
    failed_crates = []
    skipped = 0
    last_name = crates_to_publish[-1][0] if crates_to_publish else ""

    for name, crate_dir, tier in crates_to_publish:
        ver = get_local_version(name, crate_dir)
        label = TIER_LABELS[tier]

        if is_published(name, ver):
            print(f"  {GREEN}✓{NC} {name} {ver} [{label}] — already published, skipping")
            skipped += 1
            continue

        if args.execute:
            print(f"  {YELLOW}▶{NC} Publishing {name} {ver} [{label}] ...")
            if cargo_publish(name):
                print(f"  {GREEN}✓{NC} {name} {ver} [{label}] — published")
                published_crates.append((name, ver))
                if name != last_name:
                    print(f"    {YELLOW}Waiting {args.wait}s for crates.io indexing...{NC}")
                    time.sleep(args.wait)
            else:
                print(f"  {RED}✗{NC} {name} {ver} [{label}] — failed")
                print()
                print(f"{RED}Fix the issue and resume with:{NC}")
                print(f"  ./scripts/publish_crates.py --from {name} --execute")
                sys.exit(1)
        elif args.dry_run:
            print(f"  {YELLOW}▶{NC} {name} {ver} [{label}] — would publish, testing build...")
            if cargo_publish(name, dry_run=True):
                print(f"  {GREEN}✓{NC} {name} {ver} [{label}] — build OK")
                published_crates.append((name, ver))
            else:
                print(f"  {RED}✗{NC} {name} {ver} [{label}] — build failed")
                failed_crates.append((name, ver))
        else:
            print(f"  {YELLOW}▶{NC} {name} {ver} [{label}] — would publish")
            published_crates.append((name, ver))

    print()
    if args.dry_run:
        print(
            f"{GREEN}Done.{NC} Would publish: {len(published_crates)}, Failed: {len(failed_crates)}, Skipped: {skipped}")
    else:
        print(f"{GREEN}Done.{NC} Published: {len(published_crates)}, Skipped: {skipped}")
    if published_crates:
        print()
        print("Would publish:" if not args.execute else "Published crates:")
        for name, ver in published_crates:
            print(f"  - {name} {ver}")
    if failed_crates:
        print()
        print(f"{RED}Failed crates:{NC}")
        for name, ver in failed_crates:
            print(f"  - {name} {ver}")
    if not args.execute and not args.dry_run and published_crates:
        print()
        print(f"{YELLOW}Add --execute to publish for real, or --dry-run to test builds.{NC}")
    if args.dry_run and failed_crates:
        sys.exit(1)


if __name__ == "__main__":
    main()
