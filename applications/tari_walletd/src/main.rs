// Copyright 2021. The Tari Project
//
// Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
// following conditions are met:
//
// 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
// disclaimer.
//
// 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
// following disclaimer in the documentation and/or other materials provided with the distribution.
//
// 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
// products derived from this software without specific prior written permission.
//
// THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
// INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
// DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
// SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
// SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
// WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
// USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{fs, panic, process};

use anyhow::Context;
use log::*;
use ootle_byte_type::ToByteType;
use serde_json::json;
use tari_common::initialize_logging;
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_app_utilities::configuration::load_configuration;
use tari_ootle_wallet_sdk::{cipher_seed::CipherSeedRestore, models::KeyBranch};
use tari_ootle_walletd::{
    cli::{Cli, Subcommand},
    config::ApplicationConfig,
    init_os_keyring_store,
    init_wallet_store,
    initialize_wallet_sdk,
    reset_wallet_passkeys,
    run_tari_ootle_walletd,
};
use tari_shutdown::Shutdown;

const LOG_TARGET: &str = "tari::wallet_daemon";

#[allow(clippy::too_many_lines)]
#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
    // Set up a panic hook which prints the default rust panic message but also exits the process. This makes a panic in
    // any thread "crash" the system instead of silently continuing.
    let default_hook = panic::take_hook();
    panic::set_hook(Box::new(move |info| {
        default_hook(info);
        process::exit(1);
    }));

    let mut cli = Cli::init();
    let config_path = cli.common.config_path();
    let cfg = load_configuration(config_path, true, &cli, Some(cli.network()))?;
    let mut config = ApplicationConfig::load_from(&cfg)?;

    config.ootle_wallet_daemon.network = cli.network();
    if let Some(password) = cli.override_keyring_password.take() {
        config.ootle_wallet_daemon.override_keyring_password = Some(password);
    }

    if config.ootle_wallet_daemon.override_keyring_password.is_none() {
        init_os_keyring_store();
    }

    match &cli.command {
        Some(Subcommand::Run) | None => run(cli, config).await?,
        Some(Subcommand::ResetPasskeys) => {
            let deleted = reset_wallet_passkeys(&config)?;
            println!(
                "Removed {deleted} WebAuthn passkey(s). Restart the wallet daemon to invalidate any active sessions \
                 and generate a new setup token."
            );
            return Ok(());
        },
        Some(Subcommand::CreateAccount {
            name,
            key_index,
            set_active,
            output_path,
        }) => {
            let wallet_store = init_wallet_store(&config)?;
            let mut sdk = initialize_wallet_sdk(&config, wallet_store)?;
            sdk.initialize_cipher_seed(
                cli.wallet_restore
                    .seed_words
                    .as_ref()
                    .map(CipherSeedRestore::FromSeedWords)
                    .unwrap_or_default(),
            )?;
            let km = sdk.key_manager_api();
            let account_address = if let Some(index) = key_index {
                km.derive_account_address(*index)?
            } else {
                km.next_account_address()?
            };

            let public_key = account_address.address.account_key().to_byte_type();
            let view_only_public_key = account_address.address.view_only_key().to_byte_type();
            let account_addr = sdk.accounts_api().derive_account_address_from_public_key(&public_key);
            let is_default = !sdk.accounts_api().has_default_account()?;
            let birthday_epoch = sdk.calculate_birthday_epoch();
            sdk.accounts_api().add_account(
                name.as_deref(),
                &account_addr,
                account_address.view_only_key_id,
                account_address.owner_key_id,
                birthday_epoch,
                false,
                is_default,
            )?;

            if *set_active && let Some(index) = account_address.owner_key_id.derived_index() {
                km.set_active_key(KeyBranch::Account, index)?;
            }

            let view_only_secret = km.get_key(account_address.view_only_key_id)?;

            let json = json!({
                "component_address": account_addr,
                "address": account_address.address.to_byte_type(),
                "account_public_key": public_key,
                "view_only_public_key": view_only_public_key,
                "view_only_private_key": hex::encode(view_only_secret.secret().as_bytes()),
                "key_index": account_address.view_only_key_id,
            });
            match output_path {
                Some(path) => {
                    let mut file = fs::File::options()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(path)
                        .context("failed to open file for writing")?;
                    serde_json::to_writer_pretty(&mut file, &json).context("failed to encode key json to file")?;
                    println!("Key written to {}", path.display());
                },
                None => {
                    println!("{}", json);
                },
            }

            return Ok(());
        },
        Some(Subcommand::NewViewableBalanceKey { key_index, output_path }) => {
            let wallet_store = init_wallet_store(&config)?;
            let mut sdk = initialize_wallet_sdk(&config, wallet_store)?;
            sdk.initialize_cipher_seed(
                cli.wallet_restore
                    .seed_words
                    .as_ref()
                    .map(CipherSeedRestore::FromSeedWords)
                    .unwrap_or_default(),
            )?;
            let km = sdk.key_manager_api();
            let key = km.get_elgamal_encrypted_view_key(*key_index)?;
            let public_key = key.to_public_key().to_byte_type();

            let json = json!({
                "viewable_balance_public_key": public_key,
                "viewable_balance_private_key": hex::encode(key.key.as_bytes()),
                "key_index": key_index,
            });
            match output_path {
                Some(path) => {
                    let mut file = fs::File::options()
                        .create(true)
                        .write(true)
                        .truncate(true)
                        .open(path)
                        .context("failed to open file for writing")?;
                    serde_json::to_writer_pretty(&mut file, &json).context("failed to encode key json to file")?;
                    println!("Key written to {}", path.display());
                },
                None => {
                    println!("{}", json);
                },
            }

            return Ok(());
        },
        Some(Subcommand::Reset { confirm }) => {
            let network = cli.network();
            if !network.is_testnet() {
                anyhow::bail!("The reset command is not available on mainnet");
            }

            let db_path = config.to_data_dir().join("wallet.sqlite");

            println!(
                "This command is intended for testnet resets only.\nIt will move the wallet database at '{}' to a \
                 .bak file, removing all accounts,\ntransactions, balances and other on-chain state.\nYour seed key \
                 is preserved in the OS keyring and can be used to recover accounts.",
                db_path.display()
            );

            if !confirm {
                use std::io::Write;
                print!("\nAre you sure you want to reset the wallet? Type 'yes' to continue: ");
                std::io::stdout().flush()?;
                let mut input = String::new();
                tokio::io::AsyncBufReadExt::read_line(&mut tokio::io::BufReader::new(tokio::io::stdin()), &mut input)
                    .await?;
                if input.trim() != "yes" {
                    println!("Reset cancelled.");
                    return Ok(());
                }
            }

            let timestamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .context("System time is before the UNIX epoch")?
                .as_secs();
            let bak_path = db_path.with_extension(format!("sqlite.{timestamp}.bak"));
            match tokio::fs::rename(&db_path, &bak_path).await {
                Ok(()) => println!(
                    "Wallet database moved to '{}'.\nThe wallet will be recreated on next startup.",
                    bak_path.display()
                ),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    println!("No wallet database found at '{}'. Nothing to reset.", db_path.display());
                },
                Err(e) => {
                    anyhow::bail!("Failed to move wallet database: {}", e);
                },
            }

            return Ok(());
        },
        Some(Subcommand::SeedWords) => {
            let wallet_store = init_wallet_store(&config)?;
            let mut sdk = initialize_wallet_sdk(&config, wallet_store)?;
            sdk.initialize_cipher_seed(
                cli.wallet_restore
                    .seed_words
                    .as_ref()
                    .map(CipherSeedRestore::FromSeedWords)
                    .unwrap_or(CipherSeedRestore::CreateNewIfRequired),
            )?;
            let seed_words = sdk
                .load_seed_words()?
                .expect("Bug: seed words were initialized however load_seed_words returned None");
            println!("{}", seed_words.join(" ").reveal())
        },
    }

    Ok(())
}

async fn run(cli: Cli, config: ApplicationConfig) -> Result<(), anyhow::Error> {
    // Remove the file if it was left behind by a previous run
    let _file = fs::remove_file(config.common.base_path.join("pid"));

    let shutdown = Shutdown::new();
    let shutdown_signal = shutdown.to_signal();

    if let Err(e) = initialize_logging(
        &cli.common.log_config_path("ootle_wallet_daemon"),
        config.common.base_path(),
        include_str!("../log4rs_sample.yml"),
    ) {
        eprintln!("{}", e);
        return Err(e.into());
    }

    info!(
        target: LOG_TARGET,
        "📂 Base directory: {}",
        config.common.base_path().display()
    );

    run_tari_ootle_walletd(config, cli.wallet_restore.seed_words.as_ref(), shutdown_signal).await
}

#[cfg(test)]
mod tests {
    #[test]
    fn log4rs_sample_config_builds_with_enabled_features() {
        let log_dir = tempfile::tempdir().unwrap();
        let yaml = include_str!("../log4rs_sample.yml").replace("{{log_dir}}", log_dir.path().to_str().unwrap());
        let config: log4rs::config::RawConfig = serde_saphyr::from_str(&yaml).unwrap();
        log4rs::config::create_raw_config(config).unwrap();
    }
}
