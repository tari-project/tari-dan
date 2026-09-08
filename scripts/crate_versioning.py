#!/usr/bin/env python3
"""
Crate versioning helper for AI agents (and humans).

Sources the publish set + tiers from `publish_crates.py` and reads the
current dep graph from `cargo metadata`, so nothing here is maintained
by hand. When a crate is added/removed in `publish_crates.py`, this
script automatically picks it up.

Usage:
    ./scripts/crate_versioning.py list [--offline]
    ./scripts/crate_versioning.py deps <crate>
    ./scripts/crate_versioning.py dependents <crate> [--transitive]
    ./scripts/crate_versioning.py impact <crate> [--breaking] [--offline]

Tier semantics (mirrors publish_crates.py). Tier 3 is the only workspace-versioned
cohort; every other tier is independently versioned:
  1 = stable/foundational, rarely changes
  2 = template authoring crates & the built-in templates
  3 = core, all share workspace.package.version (move together)
  4 = wallet (SDK, clients, storage), decoupled from the core version

SemVer rules for 0.y.z crates:
  patch (0.y.z -> 0.y.(z+1)) — non-breaking. Dependents auto-pick-up via ^0.y.
  minor (0.y.z -> 0.(y+1).0) — breaking. Every direct dependent must update
                                its pin and republish.

A breaking bump cascades through *public* dependents: if a crate re-exposes a
minor-bumped dep in its public API, that dep's break is also a break of this
crate, so it must bump minor too (and that, in turn, can promote its own public
dependents). Deps are treated as public by default — the safe, never-under-bump
assumption. IMPL_DETAIL_DEPS records audited exceptions that are
implementation-detail only and so need just a patch + pin update.

That cascade says which crates a change is breaking FOR. It does not say which
ones still need a version edit, and the two differ whenever a release is
pending: a version that was never published carries no ^0.y pins, so nothing can
be broken by changing what it contains. A crate whose working-tree version is
already an unreleased minor ahead of crates.io therefore needs nothing — its
pending release absorbs the change and already announces it.

So every bump is checked against the crates.io sparse index before it is
printed, and a crate is only asked to move when its working-tree version is
genuinely taken (or its pending bump is too small to break the pin it needs to).
Pass --offline to skip the lookup; the output then falls back to listing the
whole cascade, bumps already made included.
"""

import argparse
import json
import subprocess
import sys
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path

# Reuse the publish list as the single source of truth.
sys.path.insert(0, str(Path(__file__).parent))
from publish_crates import (  # type: ignore[import-not-found]
    CRATES,
    TIER_LABELS,
    published_versions,
)

REPO_ROOT = Path(__file__).resolve().parent.parent
PUBLISH_SET = {name for name, _, _ in CRATES}
TIER_OF = {name: tier for name, _, tier in CRATES}

RED = "\033[0;31m"
GREEN = "\033[0;32m"
YELLOW = "\033[1;33m"
CYAN = "\033[0;36m"
BOLD = "\033[1m"
NC = "\033[0m"

# Public-dependency model.
#
# When a dependency makes a breaking (0.x minor) change, a crate must also bump
# minor IF that dependency is *public* — i.e. its types appear in the crate's own
# public API (re-exported, in a public fn signature, a public field, …). Then the
# dep's break is a break of this crate too, and it cascades to the crate's own
# public dependents.
#
# Every publish-set dependency is treated as PUBLIC by default. That is the safe
# default: it can only ever over-bump (a needless minor), never under-bump. The
# under-bump is the real hazard — it ships a breaking change as a patch and
# silently breaks consumers pinning "^0.y" (exactly how tari_indexer_client 0.32.2
# shipped a tari_engine_types 0.33->0.34 break as a patch).
#
# IMPL_DETAIL_DEPS is the deny-list of edges we have AUDITED and confirmed are
# implementation-detail only (the dep's types never reach the crate's public API).
# Such an edge needs only a patch + pin update when the dep bumps, not a minor.
#
#   Add an edge here ONLY after verifying the dep does not appear in the crate's
#   public API. Omitting an edge is safe (conservative minor); wrongly adding one
#   re-introduces the under-bump bug.
#
#   Example — a crate that uses tari_bor purely for internal encode/decode:
#     IMPL_DETAIL_DEPS = {"tari_ootle_wallet_storage_sqlite": {"tari_bor"}}
#
# {crate_name: {dep_name, ...}}
IMPL_DETAIL_DEPS = {}


def is_public_dep(crate, dep):
    """True unless the (crate -> dep) edge is an audited implementation-detail."""
    return dep not in IMPL_DETAIL_DEPS.get(crate, set())


# ---------------------------------------------------------------------------
# Registry state — what is actually published, vs what the tree says.
# ---------------------------------------------------------------------------

REGISTRY_WORKERS = 8


def vkey(version: str):
    """Sortable key for a version, ignoring any pre-release/build suffix."""
    core = version.split("-", 1)[0].split("+", 1)[0]
    parts = []
    for part in core.split("."):
        parts.append(int(part) if part.isdigit() else 0)
    while len(parts) < 3:
        parts.append(0)
    return tuple(parts[:3])


def breaks_pin(newer: str, older: str) -> bool:
    """True if `newer` falls outside a caret pin on `older`.

    Cargo's caret rules make the leftmost non-zero component the compatibility
    one: ^1.2 admits 1.3, ^0.2 admits 0.2.9 but not 0.3, ^0.0.2 admits nothing
    but 0.0.2.
    """
    a, b = vkey(newer), vkey(older)
    if a[0] != b[0]:
        return True
    if a[0] == 0 and a[1] != b[1]:
        return True
    if a[0] == 0 and a[1] == 0 and a[2] != b[2]:
        return True
    return False


@dataclass
class Release:
    """A crate's working-tree version set against what crates.io holds."""

    name: str
    local: str
    # [(version, yanked), ...] — or None when the lookup failed or was skipped.
    published: "list | None"

    @property
    def known(self) -> bool:
        return self.published is not None

    @property
    def latest(self):
        """Highest non-yanked published version, or None if there is none.

        Yanked versions are excluded because this is the version dependents
        resolve to; `taken` keeps them because their numbers stay spent.
        """
        live = [v for v, yanked in self.published or () if not yanked]
        return max(live, key=vkey, default=None)

    @property
    def taken(self) -> bool:
        """The tree's own version is spent — a release must carry a new number."""
        return any(v == self.local for v, _ in self.published or ())

    def carries(self, kind: str) -> bool:
        """Is a `kind`-sized bump already made but unreleased?

        A version nobody could depend on cannot break anybody, so an unpublished
        local version that is far enough ahead of crates.io has already done the
        job: the pending release ships the change under a number that announces
        it. Unknown registry state answers False, so the caller asks for the bump
        rather than skipping one that is needed.
        """
        if not self.known or self.taken:
            return False
        if self.latest is None:
            return True  # never published — the first release announces everything
        if vkey(self.local) <= vkey(self.latest):
            return False  # tree is level with or behind the registry
        return breaks_pin(self.local, self.latest) if kind == "minor" else True

    def describe(self) -> str:
        if not self.known:
            return "registry unknown"
        if self.taken:
            return f"{self.local} is on crates.io"
        if self.latest is None:
            return "never published"
        return f"{self.local} pending, crates.io has {self.latest}"


def fetch_releases(versions: dict, offline: bool = False) -> dict:
    """Look up every crate's published versions, concurrently."""
    if offline:
        return {n: Release(n, v, None) for n, v in versions.items()}
    names = list(versions)
    with ThreadPoolExecutor(max_workers=REGISTRY_WORKERS) as pool:
        found = dict(zip(names, pool.map(published_versions, names)))
    return {n: Release(n, versions[n], found[n]) for n in names}


def registry_caveat(releases: dict, offline: bool) -> str:
    """A warning line when the bumps below are not registry-checked, else ''."""
    if offline:
        return (f"{YELLOW}Registry check skipped (--offline) — bumps already made "
                f"but unreleased are listed as if still needed.{NC}")
    unknown = sorted(n for n, r in releases.items() if not r.known)
    if not unknown:
        return ""
    shown = ", ".join(unknown[:3]) + (f", +{len(unknown) - 3} more" if len(unknown) > 3 else "")
    return (f"{YELLOW}crates.io lookup failed for {len(unknown)} crate(s) ({shown}) — "
            f"treating them as published, so their bumps are listed.{NC}")


def cargo_metadata():
    result = subprocess.run(
        ["cargo", "metadata", "--no-deps", "--format-version", "1"],
        cwd=REPO_ROOT, capture_output=True, text=True,
    )
    if result.returncode != 0:
        sys.exit(f"cargo metadata failed:\n{result.stderr}")
    return json.loads(result.stdout)


def build_graph():
    """Return (versions, deps, dev_deps) keyed by crate name, restricted to PUBLISH_SET."""
    md = cargo_metadata()
    versions, deps, dev_deps = {}, {}, {}
    for pkg in md["packages"]:
        name = pkg["name"]
        if name not in PUBLISH_SET:
            continue
        versions[name] = pkg["version"]
        normal, dev = set(), set()
        for d in pkg["dependencies"]:
            if d["name"] not in PUBLISH_SET:
                continue
            kind = d.get("kind") or "normal"
            if kind == "dev":
                dev.add(d["name"])
            else:
                normal.add(d["name"])
        # If something appears both normal and dev, treat as normal.
        dev -= normal
        deps[name] = normal
        dev_deps[name] = dev
    missing = PUBLISH_SET - versions.keys()
    if missing:
        sys.exit(f"Publish-set crates missing from cargo metadata: {sorted(missing)}")
    return versions, deps, dev_deps


def reverse(deps):
    rev = {n: set() for n in deps}
    for n, ds in deps.items():
        for d in ds:
            rev.setdefault(d, set()).add(n)
    return rev


def transitive_dependents(seed: str, rev_deps):
    """Forward-closure of dependents (excludes dev edges)."""
    seen, stack = set(), [seed]
    while stack:
        cur = stack.pop()
        for r in rev_deps.get(cur, ()):
            if r not in seen:
                seen.add(r)
                stack.append(r)
    return seen


def cmd_list(args):
    versions, _, _ = build_graph()
    releases = fetch_releases(versions, args.offline)
    print(f"{BOLD}Publish order (from publish_crates.py):{NC}")
    for name, crate_dir, tier in CRATES:
        ver = versions[name]
        r = releases[name]
        if not r.known:
            plain, colour = "", ""
        elif r.taken:
            plain, colour = "released", GREEN
        else:
            plain, colour = f"pending (crates.io: {r.latest or 'never'})", YELLOW
        # Pad on the visible text: the colour escapes would otherwise count
        # toward the field width and stagger the column.
        status = f"{colour}{plain}{NC}" + " " * max(0, 34 - len(plain))
        print(f"  {name:38} {ver:8}  [{TIER_LABELS[tier]:11}]  {status}{crate_dir}")
    caveat = registry_caveat(releases, args.offline)
    if caveat:
        print(caveat)


def cmd_deps(args):
    versions, deps, dev_deps = build_graph()
    if args.crate not in PUBLISH_SET:
        sys.exit(f"{RED}{args.crate} is not in the publish set.{NC}")
    print(f"{BOLD}{args.crate} {versions[args.crate]} [{TIER_LABELS[TIER_OF[args.crate]]}]{NC}")
    print(f"{BOLD}Direct deps in the publish set:{NC}")
    for d in sorted(deps[args.crate]):
        print(f"  -> {d} {versions[d]} [{TIER_LABELS[TIER_OF[d]]}]")
    if dev_deps[args.crate]:
        print(f"{BOLD}Dev-only deps (do not force downstream bumps):{NC}")
        for d in sorted(dev_deps[args.crate]):
            print(f"  -> {d} {versions[d]} [{TIER_LABELS[TIER_OF[d]]}] [dev]")


def cmd_dependents(args):
    versions, deps, dev_deps = build_graph()
    if args.crate not in PUBLISH_SET:
        sys.exit(f"{RED}{args.crate} is not in the publish set.{NC}")
    rev = reverse(deps)
    rev_dev = reverse(dev_deps)
    print(f"{BOLD}{args.crate} {versions[args.crate]} [{TIER_LABELS[TIER_OF[args.crate]]}]{NC}")
    direct = sorted(rev.get(args.crate, ()))
    print(f"{BOLD}Direct dependents (normal):{NC}" + (" (none)" if not direct else ""))
    for d in direct:
        print(f"  <- {d} {versions[d]} [{TIER_LABELS[TIER_OF[d]]}]")
    direct_dev = sorted(rev_dev.get(args.crate, ()))
    if direct_dev:
        print(f"{BOLD}Direct dependents (dev-only — do not need to bump):{NC}")
        for d in direct_dev:
            print(f"  <- {d} {versions[d]} [{TIER_LABELS[TIER_OF[d]]}] [dev]")
    if args.transitive:
        trans = transitive_dependents(args.crate, rev) - set(direct) - {args.crate}
        print(f"{BOLD}Transitive dependents (via normal deps):{NC}" + (" (none)" if not trans else ""))
        for d in sorted(trans):
            print(f"  <~ {d} {versions[d]} [{TIER_LABELS[TIER_OF[d]]}]")


def classify(name: str, kind: str, releases: dict) -> str:
    """How much of a `kind` bump is still outstanding for this crate.

      covered — the bump is already made and unreleased; nothing to do.
      short   — a bump is pending but too small to break the pin it must break.
      needed  — the tree's version is spent (or unknown); it has to move.
    """
    r = releases[name]
    if r.carries(kind):
        return "covered"
    if r.known and not r.taken and r.latest and vkey(r.local) > vkey(r.latest):
        return "short"
    return "needed"


def cmd_impact(args):
    versions, deps, dev_deps = build_graph()
    target = args.crate
    if target not in PUBLISH_SET:
        sys.exit(f"{RED}{target} is not in the publish set.{NC}")
    rev = reverse(deps)
    rev_dev = reverse(dev_deps)
    cur_ver = versions[target]
    target_tier = TIER_OF[target]

    print(f"{BOLD}Impact analysis: {target} {cur_ver} [{TIER_LABELS[target_tier]}]{NC}")

    if not args.breaking:
        rel = fetch_releases({target: cur_ver}, args.offline)[target]
        if rel.carries("patch"):
            print(f"{GREEN}Non-breaking (patch) change — no version edit needed.{NC}")
            print(f"  {target} {rel.describe()}, so the pending release carries it.")
        else:
            print(f"{GREEN}Non-breaking (patch) bump.{NC}")
            print(f"  {target}: {cur_ver} -> {bump(cur_ver, 'patch')}")
        print(f"  Dependents auto-pick-up via ^0.y constraints — no republish needed")
        print(f"  unless a dependent wants to ship the fix.")
        return

    releases = fetch_releases(versions, args.offline)
    target_state = classify(target, "minor", releases)
    if target_state == "covered":
        print(f"{GREEN}Breaking (minor) change — {target} {releases[target].describe()}, "
              f"so its own bump is already made.{NC}")
    else:
        print(f"{YELLOW}Breaking (minor) bump: {cur_ver} -> {bump(cur_ver, 'minor')}{NC}")
    caveat = registry_caveat(releases, args.offline)
    if caveat:
        print(caveat)
    print()

    tier3_crates = {n for n, t in TIER_OF.items() if t == 3}

    # Fixed-point: which crates must republish at a new (minor) version, vs which
    # only need a pin update + patch.
    #   - target is "minor" (the breaking change).
    #   - Any tier-3 crate that bumps at all forces the entire tier-3 cohort to
    #     bump minor (workspace.package.version moves them together).
    #   - Any crate with a PUBLIC dep on a minor-bumped crate is itself a breaking
    #     change (the changed types reach its public API) and joins the set. This
    #     cascades: a crate promoted to minor can in turn promote its own public
    #     dependents.
    #
    # This set is who the change is breaking FOR. Whether each of them still needs
    # a version edit is a separate question, answered against the registry below —
    # a crate already sitting on an unreleased minor has nothing left to do.
    minor_set = {target}
    while True:
        changed = False
        # 1. workspace rollup
        if minor_set & tier3_crates and not tier3_crates.issubset(minor_set):
            minor_set |= tier3_crates
            changed = True
        # 2. public-dependency cascade
        for c, ds in deps.items():
            if c in minor_set:
                continue
            if any(is_public_dep(c, d) for d in ds & minor_set):
                minor_set.add(c)
                changed = True
        if not changed:
            break

    # Pin-update set: non-tier-3 crates left with a normal dep into the minor set
    # *only* through impl-detail edges (any public edge would have promoted them
    # into minor_set above). They recompile and republish a PATCH — the changed
    # types don't reach their public API — and patch bumps are absorbed by ^0.y
    # pins, so they don't cascade further.
    pin_update_set = {
        c for c, ds in deps.items()
        if c not in minor_set and TIER_OF[c] != 3 and ds & minor_set
    }

    state = {c: classify(c, "minor", releases) for c in minor_set}
    state.update({c: classify(c, "patch", releases) for c in pin_update_set})
    # Tier-3 members are reported by the cohort block below, which owns their
    # shared version; listing them again as individually covered double-counts.
    covered = sorted(c for c, st in state.items()
                     if st == "covered" and TIER_OF[c] != 3)

    # Render tier-3 cohort. The cohort shares one version, so it moves as soon as
    # any single member still owes a bump.
    t3_in_play = sorted(minor_set & tier3_crates)
    t3_moving = [c for c in t3_in_play if state[c] != "covered"]
    if t3_in_play:
        ws_ver = workspace_version()
        print(f"{BOLD}Tier 3 (core) — the workspace cohort:{NC}")
        if not t3_moving:
            print(f"  {GREEN}No rollup needed{NC} — [workspace.package].version {ws_ver} is "
                  f"already an unreleased breaking bump ahead of crates.io.")
            for c in t3_in_play:
                print(f"    {c:38} {releases[c].describe()}")
        else:
            new_ws = bump(ws_ver, "minor")
            print(f"  Set [workspace.package].version = \"{new_ws}\" in root Cargo.toml.")
            print(f"  Update every tier-3 pin in [workspace.dependencies] "
                  f"from \"{trim(ws_ver)}\" -> \"{trim(new_ws)}\".")
            if 0 < len(t3_moving) < len(t3_in_play):
                print(f"  The whole cohort moves because {len(t3_moving)} member(s) "
                      f"still owe a bump:")
                for c in t3_moving:
                    print(f"    {c:38} ({releases[c].describe()})")
            for c in sorted(tier3_crates):
                marker = "*" if c == target else " "
                print(f"  {marker} {c} {versions[c]} -> {new_ws}")
        print()

    # Render independent (non-core) minor bumps that are still outstanding.
    non_t3_minor = sorted(c for c in minor_set
                          if TIER_OF[c] != 3 and state[c] != "covered")
    if non_t3_minor:
        print(f"{BOLD}Independent (non-core) — minor (breaking) bump required:{NC}")
        for c in non_t3_minor:
            marker = "*" if c == target else " "
            line = f"  {marker} {c} {versions[c]} -> {bump(versions[c], 'minor')}"
            if c == target:
                print(f"{line}  (the changed crate)")
            else:
                causes = sorted(d for d in deps[c] & minor_set if is_public_dep(c, d))
                print(f"{line}  (re-exposes {', '.join(causes)} in public API)")
            if state[c] == "short":
                print(f"      {YELLOW}{versions[c]} is pending but does not break "
                      f"^{releases[c].latest} — promote it.{NC}")
            elif releases[c].known:
                print(f"      {releases[c].describe()}")
        print()

    # Render pin-update set (independent crates that must republish).
    pin_independent = sorted(c for c in pin_update_set
                             if TIER_OF[c] != 3 and state[c] != "covered")
    if pin_independent:
        print(f"{BOLD}Independent (non-core) — recompile & republish a PATCH "
              f"(deps are impl-detail, not re-exposed):{NC}")
        for c in pin_independent:
            cur = versions[c]
            # Which deps of c are bumping?
            bumping_deps = sorted(deps[c] & minor_set)
            pin_hint = ", ".join(f"{d}=\"{trim(bump(versions[d], 'minor'))}\""
                                  for d in bumping_deps[:4])
            if len(bumping_deps) > 4:
                pin_hint += f", … (+{len(bumping_deps) - 4} more)"
            print(f"  {c} {cur} -> {bump(cur, 'patch')} [{TIER_LABELS[TIER_OF[c]]}]")
            print(f"    pins:  {pin_hint}")
        print()

    # Crates the change breaks for, whose bump is already made and unreleased.
    if covered:
        print(f"{GREEN}Already covered by an unreleased bump ({len(covered)}) — "
              f"no action:{NC}")
        for c in covered:
            tag = " [tier-3 cohort]" if TIER_OF[c] == 3 else ""
            print(f"  {c:38} {releases[c].describe()}{tag}")
        print(f"  A ^0.y pin cannot exist on a version that was never published, so "
              f"the pending")
        print(f"  release absorbs the change under a number that already announces it.")
        print()

    # Dev-only callouts on the target (informational).
    direct_dev_dependents = sorted(rev_dev.get(target, ()))
    if direct_dev_dependents:
        print(f"{CYAN}Dev-only dependents on {target} "
              f"(no version bump required for these):{NC}")
        for c in direct_dev_dependents:
            print(f"  {c} {versions[c]} [{TIER_LABELS[TIER_OF[c]]}] [dev]")
        print()

    # Suggested workflow.
    if not t3_moving and not non_t3_minor and not pin_independent:
        print(f"{BOLD}{GREEN}No version changes required.{NC}")
        print(f"  Every crate this change breaks is already on an unreleased bump.")
        if args.offline:
            print(f"  (…as far as --offline can tell; re-run without it to confirm.)")
        return

    print(f"{BOLD}Suggested workflow:{NC}")
    step = 1
    if t3_moving:
        print(f"  {step}. Bump workspace.package.version in root Cargo.toml.")
        step += 1
        print(f"  {step}. Update tier-3 pins in [workspace.dependencies].")
        step += 1
    if non_t3_minor:
        print(f"  {step}. Minor-bump these independent crates in their own Cargo.toml,")
        print(f"     and update each one's pin in [workspace.dependencies]:")
        for c in non_t3_minor:
            print(f"       {c} -> {bump(versions[c], 'minor')}")
        step += 1
    if pin_independent:
        print(f"  {step}. Patch-bump these and update their pins:")
        for c in pin_independent:
            print(f"       {c} -> {bump(versions[c], 'patch')}")
        step += 1
    print(f"  {step}. cargo +nightly-2025-12-05 fmt --all, then "
          f"./scripts/publish_crates.py --dry-run, then --execute.")


def workspace_version() -> str:
    root = REPO_ROOT / "Cargo.toml"
    for line in root.read_text().splitlines():
        line = line.strip()
        if line.startswith("version") and "=" in line:
            return line.split("=", 1)[1].strip().strip('"')
    sys.exit("Could not find [workspace.package].version in root Cargo.toml")


def bump(version: str, kind: str) -> str:
    parts = [int(p) for p in version.split(".")]
    while len(parts) < 3:
        parts.append(0)
    major, minor, patch = parts[:3]
    if kind == "patch":
        return f"{major}.{minor}.{patch + 1}"
    if kind == "minor":
        if major == 0:
            # 0.y.z -> 0.(y+1).0 is the breaking bump under Cargo's pre-1.0 rules.
            return f"0.{minor + 1}.0"
        return f"{major}.{minor + 1}.0"
    sys.exit(f"unknown bump kind: {kind}")


def trim(version: str) -> str:
    """0.32.0 -> 0.32 (the form used in [workspace.dependencies] pins)."""
    parts = version.split(".")
    if parts[0] == "0":
        return ".".join(parts[:2])
    return parts[0]


def main():
    p = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sp = p.add_subparsers(dest="cmd", required=True)

    OFFLINE_HELP = ("Skip the crates.io lookup. Bumps already made but not yet "
                    "released can then no longer be told apart from ones still owed.")

    pl = sp.add_parser("list", help="Show the publish set with versions and tiers.")
    pl.add_argument("--offline", action="store_true", help=OFFLINE_HELP)
    pl.set_defaults(func=cmd_list)

    pd = sp.add_parser("deps", help="What does this crate depend on (in the publish set)?")
    pd.add_argument("crate")
    pd.set_defaults(func=cmd_deps)

    pr = sp.add_parser("dependents", help="What depends on this crate (in the publish set)?")
    pr.add_argument("crate")
    pr.add_argument("--transitive", action="store_true", help="Include indirect dependents.")
    pr.set_defaults(func=cmd_dependents)

    pi = sp.add_parser("impact", help="Who needs to bump if this crate bumps?")
    pi.add_argument("crate")
    pi.add_argument("--breaking", action="store_true",
                    help="Treat the change as a breaking (minor) bump rather than a patch.")
    pi.add_argument("--offline", action="store_true", help=OFFLINE_HELP)
    pi.set_defaults(func=cmd_impact)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
