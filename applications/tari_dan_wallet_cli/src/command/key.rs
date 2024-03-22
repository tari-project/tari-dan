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

use clap::Subcommand;
use tari_common_types::types::PublicKey;
use tari_wallet_daemon_client::{types::KeyBranch, WalletDaemonClient};

use crate::{table::Table, table_row};

#[derive(Debug, Subcommand, Clone)]
pub enum KeysSubcommand {
    #[clap(alias = "create")]
    New,
    List,
    Use {
        index: u64,
    },
}

impl KeysSubcommand {
    pub async fn handle(self, mut client: WalletDaemonClient) -> anyhow::Result<()> {
        #[allow(clippy::enum_glob_use)]
        use KeysSubcommand::*;
        match self {
            New => {
                let key = client.create_key(KeyBranch::Transaction).await?;
                println!("New key pair {} created", key.public_key);
            },
            List => {
                let resp = client.list_keys(KeyBranch::Transaction).await?;
                if resp.keys.is_empty() {
                    println!("No keys found. Use 'keys create' to create a new key pair");
                    return Ok(());
                }
                print_keys(resp.keys);
            },
            Use { index } => {
                let resp = client.set_active_key(index).await?;
                println!("Key {} ({}) is now active", index, resp.public_key);

                let resp = client.list_keys(KeyBranch::Transaction).await?;
                print_keys(resp.keys);
            },
        }
        Ok(())
    }
}

fn print_keys(keys: Vec<(u64, PublicKey, bool)>) {
    println!("Key pairs:");
    println!();

    let mut table = Table::new();
    table.set_titles(vec!["Index", "Public Key", "Active"]);
    for (index, key, is_active) in keys {
        table.add_row(table_row![index, key, if is_active { "✅" } else { "" }]);
    }
    table.print_stdout();
}
