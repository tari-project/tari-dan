//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Duration,
};

use ootle_byte_type::ToByteType;
use rand::RngExt;
use reqwest::Client;
use serde_json::json;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_indexer_client::types::{GetSubstateRequest, ListUtxosRequest};
use tari_ootle_common_types::{
    displayable::Displayable,
    engine_types::published_template::PublishedTemplateAddress,
    optional::Optional,
};
use tari_ootle_transaction::{Epoch, Network, Transaction, args};
use tari_ootle_wallet_sdk::{
    apis::{
        confidential_transfer::UtxoInputSelection,
        stealth_transfer::{TransferFeeParams, TransferOutput},
    },
    crypto::{memo::Memo, pay_to::PayTo},
    models::{AccountWithAddress, KeyBranch, KeyId, KeyType},
};
use tari_ootle_walletd_client::{
    WalletDaemonClient,
    types::{
        AccountsAssociateStealthResourceRequest,
        AccountsCreateFreeTestCoinsRequest,
        AccountsCreateRequest,
        AccountsCreateStealthTransferStatementRequest,
        AccountsGetBalancesRequest,
        AuthCredentials,
        AuthLoginRequest,
        InputSelection,
        StealthTransfer,
        StealthTransferRequest,
        StealthUtxosDecryptValueRequest,
        TransactionSubmitRequest,
        TransactionWaitResultRequest,
        TransferStatementRequest,
    },
};
use tari_template_lib_types::{
    Amount,
    ComponentAddress,
    ResourceAddress,
    UtxoId,
    amount,
    constants::TARI_TOKEN,
    metadata,
};
use tokio::time::sleep;

use crate::{coin::Coin, config::Config};

pub struct Wallet {
    pub name: String,
    pub client: WalletDaemonClient,
    pub network: Network,
}

/// Validity window stamped on simulated transactions. Well within any network's ceiling, and long
/// enough that a slow simulation run never has a transaction expire mid-flight.
const SIM_TRANSACTION_VALIDITY_EPOCHS: u64 = 10;

impl Wallet {
    /// A validity window ending [`SIM_TRANSACTION_VALIDITY_EPOCHS`] past the daemon's current epoch.
    pub async fn max_epoch(&self) -> anyhow::Result<Epoch> {
        let settings = self.client.clone().get_settings().await?;
        let current_epoch = settings
            .current_epoch
            .ok_or_else(|| anyhow::anyhow!("Wallet daemon could not reach its indexer, so the epoch is unknown"))?;
        Ok(Epoch(current_epoch.as_u64() + SIM_TRANSACTION_VALIDITY_EPOCHS))
    }

    pub async fn connect(name: String, address: &str) -> anyhow::Result<Self> {
        let mut client = WalletDaemonClient::connect(address, None)?;
        let resp = client
            .auth_request(AuthLoginRequest {
                permissions: vec!["admin".parse()?],
                credentials: AuthCredentials::None,
            })
            .await?;
        client.set_auth_token(resp.token);
        let settings = client.get_settings().await?;
        Ok(Self {
            name,
            client,
            network: settings.network.byte.try_into()?,
        })
    }
}

pub struct TrafficSim {
    config: Config,
    wallets: Vec<Wallet>,
    accounts: Vec<AccountWithAddress>,
}

impl TrafficSim {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            wallets: Vec::new(),
            accounts: Vec::new(),
        }
    }

    pub fn wallet_and_account_iter(&self) -> impl Iterator<Item = (&Wallet, &AccountWithAddress)> {
        self.wallets.iter().zip(self.accounts.iter())
    }

    pub async fn connect_exchange_wallet(&self) -> anyhow::Result<Wallet> {
        Wallet::connect("Exchange".to_string(), &self.config.exchange_wallet_url).await
    }

    pub async fn connect_indexer_client(
        &self,
    ) -> anyhow::Result<tari_indexer_client::rest_api_client::IndexerRestApiClient> {
        let mut exchange_wallet = self.connect_exchange_wallet().await?;
        let indexer_url = exchange_wallet.client.get_settings().await?.indexer_url;
        let client = tari_indexer_client::rest_api_client::IndexerRestApiClient::connect(indexer_url)?;
        Ok(client)
    }

    pub fn wallets(&self) -> &Vec<Wallet> {
        &self.wallets
    }

    pub async fn connect_to_wallets(&mut self) -> anyhow::Result<()> {
        for wallet in &self.config.wallets {
            let wallet = Wallet::connect(wallet.name.clone(), wallet.url.as_str()).await?;
            self.wallets.push(wallet);
        }
        Ok(())
    }

    pub async fn get_wallets_from_swarm(swarm_url: &str) -> anyhow::Result<Vec<Wallet>> {
        #[derive(Debug, Clone, serde::Deserialize)]
        pub struct InstanceInfo {
            #[allow(dead_code)]
            pub id: u64,
            pub name: String,
            pub ports: HashMap<String, u16>,
            #[allow(dead_code)]
            pub settings: HashMap<String, String>,
            #[allow(dead_code)]
            pub base_path: PathBuf,
            #[allow(dead_code)]
            pub instance_type: String,
            pub is_running: bool,
        }

        #[derive(serde::Deserialize, Debug)]
        struct SwarmResponse {
            pub instances: Vec<InstanceInfo>,
        }

        let response = Client::new()
            .post(swarm_url)
            .json(&json!({
                "jsonrpc": "2.0",
                "method": "list_instances",
                "params": { "by_type": "TariWalletDaemon" },
                "id": 1,
            }))
            .send()
            .await?;

        let swarm_response: serde_json::Value = response.json().await?;
        let resp = serde_json::from_value::<SwarmResponse>(
            swarm_response
                .get("result")
                .ok_or_else(|| anyhow::anyhow!("No result field in swarm response"))?
                .clone(),
        )?;
        let addrs = resp.instances.into_iter().filter_map(|instance| {
            if !instance.is_running {
                log::info!("WARN: stopped wallet instance: {}", instance.name);
            }
            let rpc_port = instance.ports.get("jrpc")?;
            Some((instance.name, format!("http://localhost:{}/json_rpc", rpc_port)))
        });

        let mut wallets = vec![];
        for (name, addr) in addrs {
            let wallet = Wallet::connect(name, &addr).await?;
            wallets.push(wallet);
        }

        Ok(wallets)
    }

    pub async fn send_random_transaction(
        &mut self,
        id: usize,
        resource_address: ResourceAddress,
        min_value: u64,
        max_value: u64,
    ) -> anyhow::Result<()> {
        if self.wallets.len() < 2 {
            return Err(anyhow::anyhow!("Need at least 2 wallets to send transactions"));
        }

        let mut rng = rand::rng();
        let sender_idx = rng.random_range(0..self.accounts.len());
        let mut receiver_idx = rng.random_range(0..self.accounts.len());

        while receiver_idx == sender_idx {
            receiver_idx = rng.random_range(0..self.accounts.len());
        }

        let sender_account = &self.accounts[sender_idx];
        let sender_wallet = &self.wallets[sender_idx];
        let receiver_address = &self.accounts[receiver_idx];
        let receiver_wallet = &self.wallets[receiver_idx];

        let amount_to_send = rng.random_range(min_value..=max_value);

        log::info!(
            "Sending {} ootle from {} to {}",
            amount_to_send,
            sender_wallet.name,
            receiver_wallet.name
        );

        let sender = &self.wallets[sender_idx];
        let mut sender_client = sender.client.clone();

        let resp = sender_client
            .accounts_stealth_transfer(StealthTransferRequest {
                owner_account: (*sender_account.component_address()).into(),
                fee_params: TransferFeeParams::new(UtxoInputSelection::PreferConfidential),
                input_selection: UtxoInputSelection::ConfidentialOnly,
                resource_address,
                badge_usage: Default::default(),
                transfers: vec![StealthTransfer {
                    destination_address: receiver_address.address().clone(),
                    blinded_output_amount: amount_to_send,
                    revealed_output_amount: Default::default(),
                    output_memo: Some(Memo::new_message(format!("Transfer {id}: {amount_to_send}")).unwrap()),
                    pay_to: PayTo::StealthPublicKey,
                    attach_sender_address: false,
                    pay_ref: None,
                }],
                max_fee: crate::MAX_FEE,
                dry_run: false,
            })
            .await?;

        log::info!(
            "Tx {} --> {}: {}",
            sender_wallet.name,
            receiver_wallet.name,
            resp.transaction_id,
        );

        log::info!(
            "send transaction of {} from {} to {}",
            amount_to_send,
            sender_account,
            receiver_address
        );

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn setup_stablecoin<P: AsRef<Path>>(
        &mut self,
        stablecoin_template: PublishedTemplateAddress,
        output_path: P,
        view_key: Option<RistrettoSecretKey>,
    ) -> anyhow::Result<()> {
        let mut exchange_wallet = self.connect_exchange_wallet().await?;
        let stablecoin_template = stablecoin_template.as_template_address();
        let account = match exchange_wallet.client.accounts_get_default().await.optional()? {
            Some(account) => AccountWithAddress::new(account.account, account.address),
            None => {
                let resp = exchange_wallet
                    .client
                    .create_account(AccountsCreateRequest {
                        account_name: Some("sim-admin".to_string()),
                        is_default: Some(true),
                        key_index: None,
                    })
                    .await?;

                exchange_wallet
                    .client
                    .create_free_test_coins(AccountsCreateFreeTestCoinsRequest {
                        account: resp.account.component_address.into(),
                        max_fee: crate::MAX_FEE,
                    })
                    .await?;
                AccountWithAddress::new(resp.account, resp.address)
            },
        };

        let view_public_key = match view_key {
            Some(secret) => RistrettoPublicKey::from_secret_key(&secret).to_byte_type(),
            None => {
                exchange_wallet
                    .client
                    .create_specific_key(KeyBranch::ElgamalEncryptionViewKey, 0)
                    .await?
                    .public_key
            },
        };

        let max_epoch = exchange_wallet.max_epoch().await?;
        let transaction = Transaction::builder(exchange_wallet.network.as_byte(), max_epoch)
            .pay_fee_from_component(*account.component_address(), crate::MAX_FEE)
            .allocate_component_address("sc")
            .call_function(stablecoin_template, "instantiate", args![
                Workspace("sc"),
                amount![10000000000000000000000000000],
                "SSC",
                metadata!(
                    "provider_name" => "Simcoin",
                    "description" => "A stablecoin for simulation purposes",
                    "url" => "https://doesntexist.com",
                ),
                8,
                view_public_key,
                None::<()>
            ])
            .put_last_instruction_output_on_workspace("admin_badge")
            .call_method(*account.component_address(), "deposit", args![Workspace("admin_badge")])
            .build_unsigned();

        let resp = exchange_wallet
            .client
            .submit_transaction(TransactionSubmitRequest {
                transaction,
                seal_signer: account
                    .account
                    .owner_key_id
                    .ok_or_else(|| anyhow::anyhow!("Exchange account has no owner key ID"))?,
                other_signers: vec![],
                signatures: vec![],
                detect_inputs: true,
                detect_inputs_use_unversioned: true,
                lock_ids: vec![],
            })
            .await?;

        let resp = exchange_wallet
            .client
            .wait_transaction_result(TransactionWaitResultRequest {
                transaction_id: resp.transaction_id,
                timeout_secs: Some(60),
            })
            .await?;

        if !resp.status.is_accepted() {
            return Err(anyhow::anyhow!(
                "Stablecoin instantiation transaction failed: {} {}",
                resp.status,
                resp.result.as_ref().and_then(|r| r.result.any_reject()).display()
            ));
        }
        let finalize = resp.result.as_ref().unwrap();

        let stable_coin = finalize
            .get_components_by_template(&stablecoin_template)
            .pop()
            .ok_or_else(|| anyhow::anyhow!("Failed to find stablecoin component in transaction finalize output"))?;

        let (resource_address, _) = finalize
            .created_resources()
            .find(|(_, res)| res.resource_type().is_stealth())
            .ok_or_else(|| anyhow::anyhow!("Failed to find stablecoin resource in transaction finalize output"))?;

        let (admin_badge, _) = finalize
            .created_resources()
            .find(|(_, res)| res.resource_type().is_non_fungible() && res.metadata().get_str("name") == Some("Admin"))
            .ok_or_else(|| {
                anyhow::anyhow!("Failed to find stablecoin admin resource in transaction finalize output")
            })?;

        let coin = Coin {
            template_address: stablecoin_template,
            component_address: stable_coin,
            resource_address,
            admin_badge,
        };

        let json = serde_json::to_string_pretty(&coin)?;

        log::info!("Stablecoin instantiated successfully");
        log::info!("writing to {}", output_path.as_ref().display());
        log::info!("{}", json);

        std::fs::write(output_path, json)?;

        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    pub async fn setup_wallet_funds(
        &mut self,
        faucet_component: ComponentAddress,
        funds_resource_address: ResourceAddress,
        admin_resource_address: ResourceAddress,
    ) -> anyhow::Result<()> {
        let mut exchange_wallet = self.connect_exchange_wallet().await?;

        let resp = exchange_wallet.client.accounts_get("sim_default".into()).await?;
        // let resp = exchange_wallet.client.accounts_get_default().await?;
        let exchange_account = AccountWithAddress::new(resp.account, resp.address);
        let exchange_account_key_id = exchange_account
            .account
            .owner_key_id
            .ok_or_else(|| anyhow::anyhow!("Exchange account has no owner key ID"))?;

        for (wallet, account) in self.wallet_and_account_iter() {
            let mut client = wallet.client.clone();
            // Resolved per wallet, not once for the run: each iteration makes several daemon
            // round-trips, so a window taken at the start can lapse before the later wallets are
            // funded.
            let max_epoch = wallet.max_epoch().await?;

            let fund_amount = 100_000_000_000u64; // 1000 coins

            client
                .associate_stealth_resource(AccountsAssociateStealthResourceRequest {
                    account: (*account.component_address()).into(),
                    resource_address: funds_resource_address,
                })
                .await?;

            let balances = client
                .get_account_balances(AccountsGetBalancesRequest {
                    account: Some((*account.component_address()).into()),
                    refresh: true,
                })
                .await
                .optional()?;
            if balances.as_ref().is_none_or(|b| {
                b.balances
                    .iter()
                    .all(|b| b.resource_address != TARI_TOKEN || b.balance.is_zero())
            }) {
                log::info!("[{}] Funding account with TARI: {}", wallet.name, account);
                client
                    .create_free_test_coins(AccountsCreateFreeTestCoinsRequest {
                        account: (*account.component_address()).into(),
                        max_fee: crate::MAX_FEE,
                    })
                    .await?;
            }
            if let Some(balance) = balances.as_ref().and_then(|b| {
                b.balances
                    .iter()
                    .find(|b| b.resource_address == funds_resource_address && b.confidential_balance > 0)
            }) {
                log::info!(
                    "[{}] Account already funded: {} (balance: {})",
                    wallet.name,
                    account,
                    balance.confidential_balance
                );
                continue;
            }

            log::info!("[{}] Converting to stealth UTXOs for account: {}", wallet.name, account);

            let transfer_resp = exchange_wallet
                .client
                .accounts_create_stealth_transfer_statement(AccountsCreateStealthTransferStatementRequest {
                    requests: vec![TransferStatementRequest {
                        // Basically ignored since we are not spending any inputs (but the account must exist)
                        sender_account: (*exchange_account.component_address()).into(),
                        resource_address: funds_resource_address,
                        // Inputs come from a bucket that we provide in the transaction, so none of the wallet's
                        // vaults/UTXOs are used
                        input_selection: InputSelection::FromBucket {
                            revealed_amount: fund_amount.into(),
                        },
                        outputs: vec![TransferOutput {
                            address: account.address().clone(),
                            revealed_amount: Amount::zero(),
                            blinded_amount: fund_amount,
                            memo: Some(Memo::new_message(format!("Initial Funding: {fund_amount}")).unwrap()),
                            pay_to: PayTo::StealthPublicKey,
                        }],
                    }],
                })
                .await?;

            let transaction = Transaction::builder(wallet.network.as_byte(), max_epoch)
                .pay_fee_from_component(*exchange_account.component_address(), crate::MAX_FEE)
                .call_method(*exchange_account.component_address(), "create_proof_by_amount", args![
                    admin_resource_address,
                    1
                ])
                .put_last_instruction_output_on_workspace("proof")
                .call_method(faucet_component, "withdraw", args![fund_amount])
                .put_last_instruction_output_on_workspace("funds")
                .stealth_transfer_with_input_bucket(
                    funds_resource_address,
                    transfer_resp.statements[0].clone(),
                    "funds",
                )
                .drop_all_proofs_in_workspace()
                .build_unsigned();

            let resp = exchange_wallet
                .client
                .submit_transaction(TransactionSubmitRequest {
                    transaction,
                    seal_signer: exchange_account_key_id,
                    other_signers: transfer_resp.signing_keys,
                    signatures: vec![],
                    detect_inputs: true,
                    detect_inputs_use_unversioned: true,
                    lock_ids: vec![transfer_resp.lock_id],
                })
                .await?;

            let resp = exchange_wallet
                .client
                .wait_transaction_result(TransactionWaitResultRequest {
                    transaction_id: resp.transaction_id,
                    timeout_secs: Some(60),
                })
                .await?;

            if !resp.status.is_accepted() {
                return Err(anyhow::anyhow!(
                    "Funding transaction failed for wallet {}: {} {}",
                    wallet.name,
                    resp.status,
                    resp.result.as_ref().and_then(|r| r.result.any_reject()).display()
                ));
            }

            log::info!(
                "[{}] Funded account {} with {} of resource {}",
                wallet.name,
                account,
                fund_amount,
                funds_resource_address
            );
        }

        Ok(())
    }

    pub async fn run_simulation(
        &mut self,
        resource_address: ResourceAddress,
        min_value: u64,
        max_value: u64,
    ) -> anyhow::Result<()> {
        self.connect_to_wallets().await?;
        self.setup_accounts().await?;

        if self.wallets.is_empty() {
            return Err(anyhow::anyhow!("No wallets found in swarm"));
        }

        log::info!("Found {} wallets", self.wallets.len());

        log::info!(
            "Starting transaction simulation (min: {}, max: {})",
            min_value,
            max_value
        );

        let mut id = 0;
        loop {
            id += 1;
            match self
                .send_random_transaction(id, resource_address, min_value, max_value)
                .await
            {
                Ok(_) => {
                    // let delay = rand::thread_rng().random_range(1..=5);
                    // sleep(Duration::from_secs(delay)).await;
                },
                Err(e) => {
                    log::info!("Transaction failed: {:?}", e);
                    sleep(Duration::from_secs(1)).await;
                },
            }
        }
    }

    pub async fn setup_accounts(&mut self) -> anyhow::Result<()> {
        for wallet in &mut self.wallets {
            let client = &mut wallet.client;
            if let Some(acc) = client.accounts_get("sim_default".into()).await.optional()? {
                log::info!(
                    "Found default account for wallet at {}: {}",
                    client.endpoint(),
                    acc.account
                );
                self.accounts.push(AccountWithAddress {
                    account: acc.account,
                    address: acc.address,
                });
            } else {
                log::info!("No default account found for wallet at {}", client.endpoint());
                let resp = client
                    .create_account(AccountsCreateRequest {
                        account_name: Some("sim_default".to_string()),
                        is_default: Some(true),
                        key_index: None,
                    })
                    .await?;
                log::info!(
                    "Created default account for wallet at {}: {}",
                    client.endpoint(),
                    resp.account
                );
                self.accounts.push(AccountWithAddress {
                    account: resp.account,
                    address: resp.address,
                });
            }
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn decrypt_utxos<P: AsRef<Path>>(
        &mut self,
        min_value: u64,
        max_value: u64,
        resource_address: ResourceAddress,
        last_id: Option<UtxoId>,
        specific_id: Option<UtxoId>,
        csv_out: Option<P>,
        view_key: Option<RistrettoSecretKey>,
    ) -> anyhow::Result<()> {
        let indexer = self.connect_indexer_client().await?;

        let resource = indexer
            .get_substate(&resource_address.into(), GetSubstateRequest {
                version: None,
                local_search_only: true,
                include_proof: false,
            })
            .await?
            .into_substate_value()
            .map(|value| value.decode())
            .transpose()?
            .and_then(|substate| substate.into_resource());

        let divisibility = resource.as_ref().map(|r| r.divisibility()).unwrap_or(6);
        let token_symbol = resource
            .as_ref()
            .and_then(|r| r.token_symbol().map(|s| s.to_string()))
            .unwrap_or_default();

        let mut wallet = self.connect_exchange_wallet().await?;

        // An externally-supplied view key is imported into the exchange wallet so the daemon can decrypt with it;
        // otherwise the wallet's own derived ElGamal view key is used.
        let view_key_id = match view_key {
            Some(secret) => wallet.client.import_key(secret, KeyType::ViewOnly).await?.key_id,
            None => KeyId::Derived {
                key_branch: KeyBranch::ElgamalEncryptionViewKey,
                index: 0,
            },
        };

        if let Some(specific_id) = specific_id {
            let resp = wallet
                .client
                .stealth_utxos_decrypt_value(StealthUtxosDecryptValueRequest {
                    resource_address,
                    ids: vec![specific_id],
                    view_key_id,
                    minimum_expected_value: Some(min_value),
                    maximum_expected_value: Some(max_value),
                })
                .await?;

            log::info!("Decrypted UTXO value:");
            for (id, value) in resp.values {
                let value = value
                    .map(Amount::from)
                    .map(|a| a.to_decimal_string(divisibility.into()));
                log::info!("{}: {} {}", id, value.display(), token_symbol);
            }

            return Ok(());
        }

        let resp = indexer
            .list_utxos(ListUtxosRequest {
                resource_address,
                limit: 1000,
                from_id: last_id,
            })
            .await?;

        log::info!(
            "Attempting to decrypt {} UTXOs within range {min_value}-{max_value}",
            resp.utxos.len()
        );
        if resp.utxos.is_empty() {
            return Ok(());
        }

        let mut utxo_ids = Vec::with_capacity(resp.utxos.len());
        for (id, utxo) in &resp.utxos {
            if !utxo.decode()?.is_burnt() {
                utxo_ids.push(*id);
            }
        }

        let mut csv = csv_out.map(csv::Writer::from_path).transpose()?;
        csv.as_mut()
            .map(|c| c.write_record(["utxo_id", "decrypted_value"]))
            .transpose()?;

        for ids in utxo_ids.chunks(10) {
            let resp = wallet
                .client
                .stealth_utxos_decrypt_value(StealthUtxosDecryptValueRequest {
                    resource_address,
                    ids: ids.to_vec(),
                    view_key_id,
                    minimum_expected_value: Some(min_value),
                    maximum_expected_value: Some(max_value),
                })
                .await?;

            for (id, value) in resp.values {
                log::info!(
                    "{}: {} {}",
                    id,
                    value
                        .map(Amount::from)
                        .map(|a| a.to_decimal_string(divisibility.into()))
                        .unwrap_or_else(|| "<Out of range>".to_string()),
                    value.map(|_| token_symbol.as_str()).unwrap_or_default()
                );

                csv.as_mut()
                    .map(|c| c.write_record(&[id.to_string(), value.map(|v| v.to_string()).unwrap_or_default()]))
                    .transpose()?;
            }
        }

        Ok(())
    }
}
