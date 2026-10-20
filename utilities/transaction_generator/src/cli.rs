//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::path::PathBuf;

use clap::{Args, Parser, Subcommand, ValueEnum};
use tari_ootle_transaction::Network;
use url::Url;

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
#[clap(propagate_version = true)]
pub struct Cli {
    #[clap(subcommand)]
    pub sub_command: SubCommand,
}

impl Cli {
    pub fn init() -> Self {
        Self::parse()
    }
}

#[derive(Subcommand, Debug)]
// The write arguments dominate the enum's size. It is parsed once at startup, so boxing it to even
// the variants out would only add an indirection.
#[allow(clippy::large_enum_variant)]
pub enum SubCommand {
    Write(WriteArgs),
    Read(ReadArgs),
}

#[derive(Args, Debug)]
pub struct WriteArgs {
    #[clap(long, short = 'n')]
    pub num_transactions: u64,
    #[clap(long, short = 'o')]
    pub output_file: PathBuf,
    #[clap(long)]
    pub overwrite: bool,
    #[clap(long, short = 'm')]
    pub manifest: Option<PathBuf>,
    /// The transactions to generate when no `--manifest` is given.
    #[clap(long, value_enum, default_value_t = Flood::Faucet, conflicts_with = "manifest")]
    pub flood: Flood,
    /// Derive each transaction's signing key from this seed and the transaction's index instead of at random.
    ///
    /// A faucet flood with a seed funds the accounts that a transfers flood with the same seed spends from. Required
    /// by `--flood transfers`.
    #[clap(long, conflicts_with = "manifest")]
    pub account_seed: Option<u64>,
    #[clap(long, short = 'a', alias = "arg")]
    pub manifest_args: Vec<String>,
    /// Template alias to address mappings used to resolve `use <alias>;` imports in the manifest,
    /// e.g. `--template MaxCompute=template_<hex>`. May be repeated.
    #[clap(long = "template", alias = "templates")]
    pub templates: Vec<String>,
    /// Extra substate inputs to declare on each generated transaction that aren't passed as `--arg`
    /// (e.g. a fee vault the manifest debits but never names). May be repeated. Accepts an
    /// unversioned id like `vault_<hex>`, a versioned `vault_<hex>:<version>`, and either form with
    /// a `:read` or `:write` suffix declaring the access intended (`vault_<hex>:?:read`); the
    /// default is `write`. Substate-typed `--arg`s are declared as inputs automatically and don't
    /// need repeating here.
    #[clap(long = "input", alias = "inputs")]
    pub inputs: Vec<String>,
    #[clap(long, alias = "args-file")]
    pub manifest_args_file: Option<PathBuf>,
    /// Load a file as a named manifest blob, e.g. `--blob template=path/to/template.wasm`. The name
    /// is resolved by `blob!(name)` / `publish_template!(name)` in the manifest and the file's bytes
    /// become the blob payload. May be repeated.
    #[clap(long = "blob", alias = "blobs")]
    pub blobs: Vec<String>,
    /// Seal each generated transaction with a fresh random keypair instead of with `--signer`, and
    /// add `--signer` as an additional signer (so its badge still authorises e.g. fee payment from
    /// its account). Requires `--signer`. Primarily useful for publishing the same template binary
    /// repeatedly: the random seal key becomes the template's author, so each publish yields a unique
    /// template address (H(author_public_key, binary_hash)) instead of colliding on a duplicate
    /// substate.
    #[clap(long, alias = "random-author")]
    pub random_signer: bool,
    #[clap(long, short = 'k', alias = "signer")]
    pub signer_secret_key: Option<String>,
    #[clap(long, short = 't')]
    pub network: Option<Network>,
    /// The last epoch the generated transactions are valid in.
    ///
    /// Every transaction carries a mandatory validity window and `max_epoch` is part of what each
    /// transaction is signed over, so it is fixed when the file is written and cannot be refreshed at
    /// submit time. Leave this unset to resolve it from `--indexer-url` as
    /// `current_epoch + --validity-epochs`, which is what you want unless generating offline.
    #[clap(long)]
    pub max_epoch: Option<u64>,
    /// Indexer to read the current epoch from when `--max-epoch` is not given.
    #[clap(long, default_value = "http://127.0.0.1:12500")]
    pub indexer_url: Url,
    /// How many epochs past the current epoch the generated transactions stay valid for. Ignored
    /// when `--max-epoch` is given. Must not exceed the network's `max_transaction_validity_epochs`.
    #[clap(long, default_value_t = 100)]
    pub validity_epochs: u64,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Flood {
    /// Every transaction claims from the builtin faucet into a new account. All of them write the faucet's vault,
    /// so each depends on the one before it.
    Faucet,
    /// Transaction `i` transfers from account `i` of `--account-seed` into a new account. No two write the same
    /// substate, so none depends on another. The accounts must first be funded by a faucet flood with the same seed
    /// and at least as many transactions.
    Transfers,
}

#[derive(Args, Debug)]
pub struct ReadArgs {
    #[clap(long, short = 'f')]
    pub input_file: PathBuf,
}
