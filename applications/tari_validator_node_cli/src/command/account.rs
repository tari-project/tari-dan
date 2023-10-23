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

use clap::{Args, Subcommand};
use tari_engine_types::instruction::Instruction;
use tari_template_builtin::ACCOUNT_TEMPLATE_ADDRESS;
use tari_template_lib::args;
use tari_validator_node_client::{types::SubmitTransactionResponse, ValidatorNodeClient};

use crate::{
    command::transaction::{submit_transaction, CommonSubmitArgs},
    key_manager::KeyManager,
};

#[derive(Debug, Subcommand, Clone)]
pub enum AccountsSubcommand {
    #[clap(alias = "new")]
    Create(CreateArgs),
}

#[derive(Debug, Args, Clone)]
pub struct CreateArgs {
    #[clap(long, alias = "dry-run")]
    pub is_dry_run: bool,
}

impl AccountsSubcommand {
    pub async fn handle<P: AsRef<Path>>(
        self,
        base_dir: P,
        mut client: ValidatorNodeClient,
    ) -> Result<(), anyhow::Error> {
        match self {
            AccountsSubcommand::Create(args) => {
                handle_create(args, base_dir, &mut client).await?;
            },
        }
        Ok(())
    }
}

pub async fn handle_create(
    args: CreateArgs,
    base_dir: impl AsRef<Path>,
    client: &mut ValidatorNodeClient,
) -> Result<SubmitTransactionResponse, anyhow::Error> {
    let key_manager = KeyManager::init(&base_dir)?;
    let key = key_manager
        .get_active_key()
        .ok_or_else(|| anyhow::anyhow!("No active key"))?;
    let owner_token = key.to_owner_token();

    let instruction = Instruction::CallFunction {
        template_address: *ACCOUNT_TEMPLATE_ADDRESS,
        function: "create".to_string(),
        args: args![owner_token],
    };

    let common = CommonSubmitArgs {
        wait_for_result: true,
        wait_for_result_timeout: Some(60),
        inputs: vec![],
        input_refs: vec![],
        version: None,
        dump_outputs_into: None,
        account_template_address: None,
        dry_run: args.is_dry_run
    };

    submit_transaction(vec![instruction], common, base_dir, client).await
}
