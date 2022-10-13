//   Copyright 2022. The Tari Project
//
//   Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//   following conditions are met:
//
//   1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//   disclaimer.
//
//   2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//   following disclaimer in the documentation and/or other materials provided with the distribution.
//
//   3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//   products derived from this software without specific prior written permission.
//
//   THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//   INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//   DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//   SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//   SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//   WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//   USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{fs, path::Path};

use clap::Subcommand;
use serde_json::json;
use tari_dan_engine::crypto::create_key_pair;
use tari_utilities::hex::Hex;

#[derive(Debug, Subcommand, Clone)]
pub enum AccountsSubcommand {
    #[clap(alias = "create")]
    New,
    List,
    Use {
        name: String,
    },
}

impl AccountsSubcommand {
    pub async fn handle<P: AsRef<Path>>(self, base_dir: P) -> anyhow::Result<()> {
        let accounts_path = base_dir.as_ref().join("accounts");
        fs::create_dir_all(&accounts_path)?;

        #[allow(clippy::enum_glob_use)]
        use AccountsSubcommand::*;
        match self {
            New => {
                let (k, p) = create_key_pair();
                fs::write(
                    accounts_path.join(format!("{}.json", p)),
                    json!({"key": k.to_hex()}).to_string(),
                )?;

                println!("New account {} created", p.to_hex());
            },
            List => {
                println!("Accounts:");
                let accounts = fs::read_dir(&accounts_path)?.filter_map(|entry| {
                    let entry = entry.ok()?;
                    let name = entry.path().file_stem()?.to_str()?.to_string();
                    Some(name)
                });
                let active_account = read_active_account(&base_dir);
                for (i, name) in accounts.enumerate() {
                    if active_account.as_ref() == Some(&name) {
                        println!("{}. (active) {}", i, name);
                    } else {
                        println!("{}. {}", i, name);
                    }
                }
            },
            Use { name } => {
                let path = accounts_path.join(format!("{}.json", name));
                if !path.exists() {
                    return Err(anyhow::anyhow!("Account {} does not exist", name));
                }
                write_active_account(base_dir, &name)?;
                println!("Account {} is now active", name);
            },
        }
        Ok(())
    }
}

fn read_active_account<P: AsRef<Path>>(base_dir: P) -> Option<String> {
    let active_account = fs::read_to_string(base_dir.as_ref().join("active_account")).ok()?;
    Some(active_account)
}

fn write_active_account<P: AsRef<Path>>(base_dir: P, name: &str) -> anyhow::Result<()> {
    fs::write(base_dir.as_ref().join("active_account"), name)?;
    Ok(())
}
