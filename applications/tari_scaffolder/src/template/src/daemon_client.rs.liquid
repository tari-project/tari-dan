//  Copyright 2023 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use multiaddr::Multiaddr;
use std::str::FromStr;
use reqwest;
use serde_json::json;
use serde_json::Value;
use tari_engine_types::instruction::Instruction;
use tari_wallet_daemon_client::types::TransactionSubmitRequest;
use tari_wallet_daemon_client::WalletDaemonClient;
use tari_wallet_daemon_client::ComponentAddressOrName;
use tari_wallet_daemon_client::types::CallInstructionRequest;
use tari_wallet_daemon_client::types::AuthLoginRequest;
 use tari_transaction::SubstateRequirement;


pub struct DaemonClient {
    endpoint: String,
    auth_token: Option<String>,
    last_id: usize,
    default_account: String
}

impl DaemonClient {
    pub(crate) fn new(endpoint: String, auth_token: Option<String>, default_account: String) -> Self {
        Self {
            endpoint,
            auth_token,
            last_id: 0,
            default_account
        }
    }

    pub async fn login(&mut self) -> String {
        let mut client =
                   WalletDaemonClient::connect(&self.endpoint, self.auth_token.clone()).unwrap();
        let r = client.auth_request(&AuthLoginRequest {
            permissions: vec!["Admin".to_string()],
            duration: None
        }).await.unwrap();

        dbg!(&r);

        r.auth_token
    }



    pub async fn submit_instruction(&mut self, instruction: Instruction, dump_buckets: bool, is_dry_run: bool, fees: u64, other_inputs: Vec<SubstateRequirement>) {
        self.submit_instructions(vec![instruction], dump_buckets, is_dry_run, fees, other_inputs).await;
    }

    pub async fn submit_instructions(&mut self, instructions: Vec<Instruction>, dump_buckets: bool, is_dry_run: bool, max_fee: u64, other_inputs: Vec<SubstateRequirement>) {
     let mut client =
            WalletDaemonClient::connect(&self.endpoint, self.auth_token.clone()).unwrap();
        //let r = client.list_keys().await;

        //dbg!(r);

           let tx = CallInstructionRequest {
            instructions,
            fee_account: ComponentAddressOrName::Name(self.default_account.clone()),
            dump_outputs_into: if dump_buckets {
                Some(ComponentAddressOrName::Name(self.default_account.clone()))
            } else {
                None
            },
            max_fee,
            inputs: other_inputs,
            override_inputs: None,
            is_dry_run,
            proof_ids: vec![],
            new_outputs: None,
            min_epoch: None,
            max_epoch: None,
        };

        let r2 = client.submit_instruction(tx).await.unwrap();


        dbg!(r2);



	    //"dump_outputs_into": self.default_account,

    }

              //  {
                //    "instruction": instruction,
                  //  "fee_account": self.last_account_name,
               //     "dump_outputs_into": self.last_account_name,
               //     "fee": 1000,
               // },
          //
}
