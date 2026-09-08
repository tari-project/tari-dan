#!/usr/bin/env python3
"""
Tests for the version arithmetic in crate_versioning.py.

No network, no cargo, no pytest — run it directly:

    python3 scripts/test_crate_versioning.py

The logic under test decides whether a release still owes a version bump. Getting
it wrong in the permissive direction ships a breaking change under a number that
^0.y pins accept, which is silent breakage for every downstream consumer, so the
cases below pin down each branch.
"""

import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))

from crate_versioning import Release, breaks_pin, classify, vkey  # noqa: E402


def rel(local, published):
    """A Release whose published list is written as ["0.1.0", "0.2.0!"] — ! = yanked."""
    if published is None:
        return Release("crate", local, None)
    return Release("crate", local, [(v.rstrip("!"), v.endswith("!")) for v in published])


def test_vkey():
    assert vkey("0.16.0") == (0, 16, 0)
    assert vkey("1.2") == (1, 2, 0)
    assert vkey("0.5.1-rc.1") == (0, 5, 1)
    assert vkey("0.5.1+build") == (0, 5, 1)
    assert vkey("0.9.0") < vkey("0.10.0"), "must compare numerically, not as strings"


def test_breaks_pin():
    # Pre-1.0: the minor is the compatibility component.
    assert breaks_pin("0.16.0", "0.15.0")
    assert not breaks_pin("0.15.1", "0.15.0")
    # Post-1.0: the major is.
    assert breaks_pin("2.0.0", "1.9.0")
    assert not breaks_pin("1.9.0", "1.2.0")
    # 0.0.z: ^0.0.z admits nothing else at all.
    assert breaks_pin("0.0.2", "0.0.1")


def test_latest_skips_yanked():
    # Yanked versions are not what dependents resolve to.
    assert rel("0.4.0", ["0.1.0", "0.3.0", "0.3.1!"]).latest == "0.3.0"
    assert rel("0.4.0", []).latest is None


def test_taken_counts_yanked():
    # A yanked number is spent — crates.io will not accept it again.
    assert rel("0.3.1", ["0.3.0", "0.3.1!"]).taken
    assert not rel("0.3.2", ["0.3.0", "0.3.1!"]).taken


def test_carries_pending_minor():
    # The whole point: an unreleased minor already announces the break.
    r = rel("0.16.0", ["0.15.0"])
    assert r.carries("minor")
    assert r.carries("patch")


def test_carries_pending_patch_is_not_enough_for_minor():
    # 0.15.1 is unreleased, but ^0.15.0 still accepts it — the break would be silent.
    r = rel("0.15.1", ["0.15.0"])
    assert not r.carries("minor")
    assert r.carries("patch")


def test_carries_released_version():
    # The tree's version is on crates.io, so any change needs a new number.
    r = rel("0.15.0", ["0.15.0"])
    assert not r.carries("minor")
    assert not r.carries("patch")


def test_carries_never_published():
    # Nothing can pin what has never been released.
    r = rel("0.1.0", [])
    assert r.carries("minor")
    assert r.carries("patch")


def test_carries_unknown_registry():
    # A failed lookup must ask for the bump, never skip one that is needed.
    r = rel("0.16.0", None)
    assert not r.carries("minor")
    assert not r.carries("patch")


def test_carries_tree_behind_registry():
    # Someone published past the tree; the local version proves nothing.
    r = rel("0.14.0", ["0.15.0"])
    assert not r.carries("minor")
    assert not r.carries("patch")


def test_classify():
    releases = {
        "covered": rel("0.16.0", ["0.15.0"]),
        "short": rel("0.15.1", ["0.15.0"]),
        "needed": rel("0.15.0", ["0.15.0"]),
        "fresh": rel("0.1.0", []),
        "unknown": rel("0.16.0", None),
    }
    assert classify("covered", "minor", releases) == "covered"
    assert classify("short", "minor", releases) == "short"
    assert classify("needed", "minor", releases) == "needed"
    assert classify("fresh", "minor", releases) == "covered"
    # An unknown registry is never reported as "short": there is no latest to
    # promote from, and "needed" is the safe answer.
    assert classify("unknown", "minor", releases) == "needed"
    # A pending patch is enough when only a patch is owed.
    assert classify("short", "patch", releases) == "covered"


def test_describe():
    assert rel("0.15.0", ["0.15.0"]).describe() == "0.15.0 is on crates.io"
    assert rel("0.16.0", ["0.15.0"]).describe() == "0.16.0 pending, crates.io has 0.15.0"
    assert rel("0.1.0", []).describe() == "never published"
    assert rel("0.1.0", None).describe() == "registry unknown"


def main():
    tests = [v for k, v in sorted(globals().items()) if k.startswith("test_")]
    for t in tests:
        t()
    print(f"{len(tests)} tests passed")


if __name__ == "__main__":
    main()
