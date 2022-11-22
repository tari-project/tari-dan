//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Claus

use std::{io::stdin, path::PathBuf};

use anyhow::anyhow;
use clap::{Args, Subcommand};

#[derive(Debug, Subcommand, Clone)]
pub enum ManifestSubcommand {
    New(NewArgs),
    Check(CheckArgs),
}

#[derive(Debug, Args, Clone)]
pub struct NewArgs {
    pub manifest: Option<PathBuf>,
}

#[derive(Debug, Args, Clone)]
pub struct CheckArgs {
    #[clap(short = 'p', long)]
    pub manifest: Option<PathBuf>,
    #[clap(short = 'g', long)]
    pub globals: Vec<String>,
}

impl ManifestSubcommand {
    pub fn handle(self) -> Result<(), anyhow::Error> {
        match self {
            ManifestSubcommand::Check(args) => {
                let contents = get_contents(args.manifest)?;
                let instructions = tari_transaction_manifest::parse_manifest(&contents, Default::default())?;
                // TODO: improve output
                println!("Instructions: {:#?}", instructions);
            },
            ManifestSubcommand::New(args) => {
                let mut out_stream = if let Some(ref path) = args.manifest {
                    Box::new(std::fs::File::create(path)?) as Box<dyn std::io::Write>
                } else {
                    Box::new(std::io::stdout()) as Box<dyn std::io::Write>
                };

                let template = include_str!("./manifest.template.rs");
                out_stream.write_all(template.as_bytes())?;
                if let Some(path) = args.manifest {
                    println!("Manifest template written to {}", path.display());
                }
            },
        }
        Ok(())
    }
}

// fn get_globals(args: &CheckArgs) -> Result<Vec<(String, String)>, anyhow::Error> {
//     let mut globals = Vec::new();
//     for global in &args.globals {
//         let mut parts = global.split('=');
//         let name = parts
//             .next()
//             .ok_or_else(|| anyhow::anyhow!("Invalid global: {}", global))?;
//         let value = parts
//             .next()
//             .ok_or_else(|| anyhow::anyhow!("Invalid global: {}", global))?;
//         globals.push((name.to_string(), value.to_string()));
//     }
//     Ok(globals)
// }

fn get_contents(manifest: Option<PathBuf>) -> Result<String, anyhow::Error> {
    match manifest {
        Some(manifest) => Ok(std::fs::read_to_string(manifest)?),
        None => stdin().lines().map(|l| l.map_err(|e| anyhow!(e))).collect(),
    }
}
