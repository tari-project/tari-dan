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

use std::path::Path;

use clap::Subcommand;

use crate::account_manager::AccountFileManager;

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
        let account_manager = AccountFileManager::init(base_dir.as_ref().to_path_buf())?;

        #[allow(clippy::enum_glob_use)]
        use AccountsSubcommand::*;
        match self {
            New => {
                let account = account_manager.create_account()?;
                println!("New account {} created", account);
            },
            List => {
                println!("Accounts:");
                for (i, account) in account_manager.all().into_iter().enumerate() {
                    if account.is_active {
                        println!("{}. (active) {}", i, account);
                    } else {
                        println!("{}. {}", i, account);
                    }
                }
            },
            Use { name } => {
                account_manager.set_active_account(&name)?;
                println!("Account {} is now active", name);
            },
        }
        Ok(())
    }
}
