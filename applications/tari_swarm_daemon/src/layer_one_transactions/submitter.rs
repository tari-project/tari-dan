//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use anyhow::{anyhow, bail};
use log::info;
use minotari_node_grpc_client::grpc;
use minotari_wallet_grpc_client::WalletGrpcClient;
use tari_crypto::tari_utilities::ByteArray;
use tari_ootle_common_types::layer_one_transaction::{
    LayerOnePayloadType,
    LayerOneTransactionDef,
    ValidatorExitParams,
    ValidatorRegistrationParams,
};
use tari_transaction_components::transaction_components::{MemoField, memo_field::TxType};

pub struct LayerOneTransactionSubmitter {
    client: WalletGrpcClient<tonic::transport::Channel>,
}

impl LayerOneTransactionSubmitter {
    pub fn new(client: WalletGrpcClient<tonic::transport::Channel>) -> Self {
        Self { client }
    }

    pub async fn submit_transaction(
        &mut self,
        transaction_def: LayerOneTransactionDef<serde_json::Value>,
    ) -> anyhow::Result<u64> {
        let proof_type = transaction_def.payload_type;
        match proof_type {
            LayerOnePayloadType::ValidatorRegistration => {
                let registration = serde_json::from_value::<ValidatorRegistrationParams>(transaction_def.payload)?;

                let resp = self
                    .client
                    .register_validator_node(grpc::RegisterValidatorNodeRequest {
                        validator_node_public_key: registration.public_key.as_bytes().to_vec(),
                        validator_node_signature: Some(grpc::Signature {
                            public_nonce: registration.signature.public_nonce().to_vec(),
                            signature: registration.signature.signature().to_vec(),
                        }),
                        validator_node_claim_public_key: registration.claim_public_key.as_bytes().to_vec(),
                        max_epoch: registration.max_epoch.as_u64(),
                        fee_per_gram: 10,
                        payment_id: MemoField::new_open(
                            format!("VN registration: {}", registration.public_key).into_bytes(),
                            TxType::ValidatorNodeRegistration,
                        )
                        .map_err(|e| anyhow!("Failed to create payment ID: {}", e))?
                        .to_bytes(),
                        sidechain_deployment_key: registration
                            .sidechain_public_key
                            .map(|key| key.to_vec())
                            .unwrap_or_default(),
                    })
                    .await?;

                let resp = resp.into_inner();
                if !resp.is_success {
                    bail!("Failed to register VN: {}", resp.failure_message);
                }
                info!(
                    "{} transaction sent successfully (tx_id={})",
                    proof_type, resp.transaction_id
                );
                Ok(resp.transaction_id)
            },
            LayerOnePayloadType::ValidatorExit => {
                let exit = serde_json::from_value::<ValidatorExitParams>(transaction_def.payload)?;

                let resp = self
                    .client
                    .submit_validator_node_exit(grpc::SubmitValidatorNodeExitRequest {
                        validator_node_public_key: exit.public_key.as_bytes().to_vec(),
                        validator_node_signature: Some(grpc::Signature {
                            public_nonce: exit.signature.public_nonce().as_bytes().to_vec(),
                            signature: exit.signature.signature().as_bytes().to_vec(),
                        }),
                        max_epoch: exit.max_epoch.as_u64(),
                        fee_per_gram: 10,
                        message: format!("Validator: Automatically submitted {proof_type} transaction").into_bytes(),
                        sidechain_deployment_key: exit
                            .sidechain_public_key
                            .map(|key| key.as_bytes().to_vec())
                            .unwrap_or_default(),
                    })
                    .await?;

                let resp = resp.into_inner();
                if !resp.is_success {
                    bail!("Failed to submit VN exit: {}", resp.failure_message);
                }
                info!(
                    "{} transaction sent successfully (tx_id={})",
                    proof_type, resp.transaction_id
                );

                Ok(resp.transaction_id)
            },
        }
    }
}
