//  Copyright 2026 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

// Builds the workspace's ootle-wasm-bor crate into ./ootle-wasm-bor, where src/utils/encoded.ts imports it from.
//
// The indexer's build.rs runs this UI's build while cargo holds the lock on the workspace target directory, so the
// wasm build gets a target directory of its own. Cargo hands build scripts the host's rustflags, which would reach
// the wasm32 link, so they are dropped.

import { spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const webUi = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repo = resolve(webUi, "../../..");

const env = { ...process.env, CARGO_TARGET_DIR: join(repo, "target", "indexer-web-ui-wasm") };
delete env.CARGO_ENCODED_RUSTFLAGS;
delete env.RUSTFLAGS;

const result = spawnSync(
  "wasm-pack",
  [
    "build",
    "--target",
    "web",
    "--release",
    "--out-dir",
    join(webUi, "ootle-wasm-bor"),
    join(repo, "crates", "ootle_wasm", "bor"),
  ],
  { env, stdio: "inherit", shell: process.platform === "win32" },
);

if (result.error) {
  console.error(`Could not run wasm-pack (is it installed?): ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
