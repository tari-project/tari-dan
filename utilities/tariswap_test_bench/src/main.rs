//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

mod accounts;
mod cli;
mod faucet;
mod runner;
mod stats;
mod tariswap;
mod templates;

use std::{fs, time::Instant};

use log::info;
use tari_template_lib::models::Amount;

use crate::{
    cli::{Cli, SubCommand},
    runner::Runner,
};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_logger();
    let cli = Cli::init();

    // Resets the key each run
    let _ignore = fs::remove_file(&cli.common.db_path);

    match cli.sub_command {
        SubCommand::Run(args) => run(cli.common, args).await?,
    }
    Ok(())
}

async fn run(cli: cli::CommonArgs, _args: cli::RunArgs) -> anyhow::Result<()> {
    let timer = Instant::now();
    let mut runner = Runner::init(cli).await?;

    info!("⏳️ Creating primary account...");
    let primary_account = runner.create_account_with_free_coins().await?;
    info!("✅ Created {}", primary_account);
    info!("⏳️ Creating other accounts...");

    let mut accounts = vec![];
    for i in 0..5 {
        // We have to break up the transactions into batches because otherwise the indexer opens too many RPC sessions
        // which then cause submit failures (starts to error at about 250 per batch)
        let start = i * 200;
        let end = start + 200;
        accounts.extend(runner.create_accounts(&primary_account, start + 1..=end).await?);
        info!("⏳️ Created 200 accounts...");
    }
    info!("✅ Created all accounts");

    info!("⏳️ Creating faucet ...");
    let faucet = runner.create_faucet(&primary_account).await?;
    info!("✅ Created faucet {}", faucet.component_address);

    info!("⏳️ Funding accounts ...");
    for batch in accounts.chunks(500) {
        runner.fund_accounts(&faucet, &primary_account, batch).await?;
        info!("✅ Funded 500 accounts");
    }

    info!("⏳️ Creating 1000 tariswap components...");
    let mut tariswaps = vec![];
    for _ in 0..4 {
        tariswaps.extend(runner.create_tariswaps(&primary_account, &faucet, 250).await?);
    }
    info!("✅ Created 1000 tariswaps");

    info!("⏳️ Adding liquidity to tariswap pools...");
    runner
        .add_liquidity(
            &tariswaps,
            &primary_account,
            &accounts,
            Amount(1000),
            Amount(100),
            &faucet,
        )
        .await?;
    info!("✅ Done adding liquidity to tariswaps");

    info!("⏳️ Submitting swaps...");
    for _ in 0..10 {
        runner
            .do_tariswap_swaps(
                &tariswaps,
                &primary_account,
                &accounts,
                Amount(1000),
                Amount(100),
                &faucet,
            )
            .await?;
        runner.log_stats();
    }
    info!("✅ Done with swaps");

    info!("✅ Test completed in {:.2?}", timer.elapsed());

    runner.log_stats();
    Ok(())
}

fn init_logger() {
    use fern::colors::ColoredLevelConfig;
    let color = ColoredLevelConfig::new()
        .info(fern::colors::Color::Green)
        .debug(fern::colors::Color::Magenta);
    fern::Dispatch::new()
        .format(move |out, message, record| out.finish(format_args!("{} {}", color.color(record.level()), message)))
        .level(log::LevelFilter::Info)
        .chain(std::io::stdout())
        .apply()
        .unwrap();
}
