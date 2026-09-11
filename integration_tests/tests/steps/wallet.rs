//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::time::Duration;

use anyhow::anyhow;
use cucumber::{gherkin::Step, given, when};
use integration_tests::{claim_proof::CucumberClaimProof, cucumber_log};
use minotari_app_grpc::{
    tari_rpc,
    tari_rpc::{GetBalanceRequest, ValidateRequest},
};
use tari_common_types::{
    burn_proof::EncodedMerkleProof as SidechainEncodedMerkleProof,
    types::{CompressedCommitment, CompressedPublicKey},
};
use tari_crypto::{
    ristretto::{CompressedRistrettoSchnorr, RistrettoSecretKey, pedersen::CompressedPedersenCommitment},
    tari_utilities::ByteArray,
};
use tari_engine_types::confidential::{AbridgedTransactionKernel, EncodedMerkleProof, MinotariBurnClaimProof};
use tari_ootle_walletd_client::types::{ClaimBurnProof, ClaimBurnProofContents};
use tari_sidechain::{AbridgedTransactionKernel as SidechainKernel, BurnClaimProof, CompleteClaimBurnProof};
use tari_template_lib_types::{
    EncryptedData,
    crypto::{PedersenCommitmentBytes, RistrettoPublicKeyBytes, Scalar32Bytes, SchnorrSignatureBytes},
};
use tari_transaction_components::{
    tari_amount::T,
    transaction_components::{MemoField, memo_field::TxType},
};
use tokio::time::sleep;

use crate::{TariWorld, spawn_minotari_wallet};

#[given(expr = "a wallet {word} connected to base node {word}")]
async fn start_wallet(world: &mut TariWorld, step: &Step, wallet_name: String, bn_name: String) {
    cucumber_log!("==== Step: {}", step.value);
    spawn_minotari_wallet(world, wallet_name, bn_name).await;
}

#[when(expr = "I burn {int}T on wallet {word} to proof {word} for account {word} using wallet daemon {word}")]
async fn when_i_burn_on_wallet_for_account(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    wallet_name: String,
    proof_name: String,
    account_name: String,
    walletd_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let walletd = world.get_wallet_daemon(&walletd_name);
    let mut walletd_client = walletd.get_authed_client().await;
    let account = walletd_client
        .accounts_get(account_name.clone().into())
        .await
        .unwrap_or_else(|e| panic!("Account {} not found: {}", account_name, e));
    let claim_public_key = account.account.owner_public_key().as_bytes().to_vec();
    burn_and_store_proof(world, amount, &wallet_name, proof_name, claim_public_key).await;
}

#[when(expr = "I burn {int}T on wallet {word} to proof {word} for wallet daemon {word}")]
async fn when_i_burn_on_wallet(
    world: &mut TariWorld,
    step: &Step,
    amount: u64,
    wallet_name: String,
    proof_name: String,
    walletd_name: String,
) {
    cucumber_log!("==== Step: {}", step.value);
    let walletd = world.get_wallet_daemon(&walletd_name);
    let mut walletd_client = walletd.get_authed_client().await;
    let account = walletd_client.accounts_get_default().await.unwrap();
    let claim_public_key = account.account.owner_public_key().as_bytes().to_vec();
    burn_and_store_proof(world, amount, &wallet_name, proof_name, claim_public_key).await;
}

async fn burn_and_store_proof(
    world: &mut TariWorld,
    amount: u64,
    wallet_name: &str,
    proof_name: String,
    claim_public_key: Vec<u8>,
) {
    let wallet = world
        .wallets
        .get(wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let burn_amount = amount * T;
    let mut wallet_client = wallet.create_client().await;
    let resp = wallet_client
        .create_burn_transaction(minotari_app_grpc::tari_rpc::CreateBurnTransactionRequest {
            amount: burn_amount.as_u64(),
            fee_per_gram: 1,
            payment_id: MemoField::new_open("Burn".as_bytes().to_vec(), TxType::Burn)
                .unwrap()
                .to_bytes(),
            claim_public_key,
            sidechain_deployment_key: vec![],
        })
        .await
        .unwrap()
        .into_inner();

    assert!(resp.is_success);

    let kernel_excess_sig_nonce = resp.kernel_excess_nonce.clone();
    let kernel_excess_sig_signature = resp.kernel_excess_signature.clone();

    integration_tests::cucumber_log!(
        "Burn transaction created with kernel_excess_sig nonce: {}, signature: {}",
        hex::encode(&kernel_excess_sig_nonce),
        hex::encode(&kernel_excess_sig_signature)
    );

    world.claim_proofs.insert(proof_name, CucumberClaimProof::Pending {
        commitment: PedersenCommitmentBytes::from_bytes(&resp.commitment).unwrap(),
        kernel_excess_sig_nonce,
        kernel_excess_sig_signature,
    });
}

#[when(expr = "I wait for proof {word} to confirm on wallet {word}")]
#[allow(clippy::too_many_lines)]
async fn when_i_wait_for_proof_to_confirm_on_wallet(
    world: &mut TariWorld,
    step: &Step,
    proof_name: String,
    wallet_name: String,
) -> anyhow::Result<()> {
    cucumber_log!("==== Step: {}", step.value);
    let proof = world.claim_proofs.get(&proof_name).unwrap_or_else(|| {
        panic!("Claim proof {} not found", proof_name);
    });

    let CucumberClaimProof::Pending { commitment, .. } = proof else {
        // Already confirmed
        return Ok(());
    };

    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let mut client = wallet.create_client().await;

    let mut attempts = 0;
    let proof_resp = loop {
        let resp = client
            .get_burn_claim_proof(tari_rpc::GetBurnClaimProofRequest {
                commitment: commitment.as_bytes().to_vec(),
            })
            .await
            .unwrap()
            .into_inner();

        cucumber_log!("Received burn claim proof response: {:?}", resp);

        let is_merkle_proof_available = resp.merkle_proof.is_some();
        if is_merkle_proof_available {
            break resp;
        }
        if attempts >= 20 {
            return Err(anyhow!(
                "Kernel Proof not available after waiting for {} attempts",
                attempts
            ));
        }
        attempts += 1;

        cucumber_log!("Kernel proof not available yet, waiting...");
        sleep(Duration::from_secs(3)).await;
    };
    // Now that the proof is confirmed, call the base node HTTP endpoint to get kernel merkle proof
    cucumber_log!("Proof confirmed! Now calling base node HTTP endpoint to get kernel merkle proof");

    let claim_proof = proof_resp
        .claim_proof
        .ok_or_else(|| anyhow!("No claim proof in response"))?;
    let ownership_proof = claim_proof
        .ownership_proof
        .ok_or_else(|| anyhow!("No ownership proof in response"))?;
    let commitment = PedersenCommitmentBytes::from_bytes(&claim_proof.commitment)
        .map_err(|e| anyhow!("commitment parse error: {e}"))?;

    let ownership_proof = SchnorrSignatureBytes::new(
        RistrettoPublicKeyBytes::from_bytes(&ownership_proof.public_nonce)
            .map_err(|e| anyhow!("sig public_nonce parse error {e}"))?,
        Scalar32Bytes::from_bytes(&ownership_proof.signature).map_err(|e| anyhow!("sig parse error {e}"))?,
    );

    let reciprocal_claim_public_key = RistrettoPublicKeyBytes::from_bytes(&claim_proof.claim_public_key)
        .map_err(|e| anyhow!("reciprocal_claim_public_key parse error {e}"))?;

    let sender_offset_public_key = RistrettoPublicKeyBytes::from_bytes(&claim_proof.sender_offset_public_key)
        .map_err(|e| anyhow!("sender_offset_public_key parse error {e}"))?;

    let kernel = proof_resp.kernel.ok_or_else(|| anyhow!("No kernel in response"))?;
    let kernel = AbridgedTransactionKernel {
        version: kernel.version as u8,
        fee: kernel.fee,
        lock_height: kernel.lock_height,
        excess: kernel
            .excess
            .as_slice()
            .try_into()
            .map_err(|e| anyhow!("excess parse error: {e}"))?,
        excess_sig: {
            let excess_sig = kernel
                .excess_sig
                .as_ref()
                .ok_or_else(|| anyhow!("No excess_sig in response"))?;

            SchnorrSignatureBytes::new(
                excess_sig
                    .public_nonce
                    .as_slice()
                    .try_into()
                    .map_err(|e| anyhow!("excess_sig parse error: {e}"))?,
                excess_sig
                    .signature
                    .as_slice()
                    .try_into()
                    .map_err(|e| anyhow!("excess_sig parse error: {e}"))?,
            )
        },
    };

    cucumber_log!(
        "DEBUG: creating confirmed proof. commitment: {}, encrypted_data: {}, reciprocal_key: {}",
        commitment,
        hex::encode(&proof_resp.encrypted_data),
        reciprocal_claim_public_key
    );
    let merkle_proof = proof_resp
        .merkle_proof
        .ok_or_else(|| anyhow!("No merkle proof in claim proof"))?;

    // Build the on-disk file format (CompleteClaimBurnProof) for auto-claim integration tests.
    // This is done before building ClaimBurnProofContents to avoid moving the local variables.
    let complete_proof = CompleteClaimBurnProof {
        claim_proof: BurnClaimProof {
            burn_public_key: CompressedPublicKey::from_canonical_bytes(reciprocal_claim_public_key.as_bytes())
                .map_err(|e| anyhow!("burn_public_key parse error for complete proof: {e}"))?,
            commitment: CompressedPedersenCommitment::from_canonical_bytes(commitment.as_bytes())
                .map_err(|e| anyhow!("commitment parse error for complete proof: {e}"))?,
            ownership_proof: CompressedRistrettoSchnorr::new(
                CompressedPublicKey::from_canonical_bytes(ownership_proof.public_nonce().as_bytes())
                    .map_err(|e| anyhow!("ownership nonce parse error for complete proof: {e}"))?,
                RistrettoSecretKey::from_canonical_bytes(ownership_proof.signature().as_bytes())
                    .map_err(|e| anyhow!("ownership sig parse error for complete proof: {e}"))?,
            ),
            encoded_merkle_proof: SidechainEncodedMerkleProof {
                block_hash: merkle_proof
                    .block_hash
                    .as_slice()
                    .try_into()
                    .map_err(|e| anyhow!("block_hash parse error for complete proof: {e}"))?,
                encoded_merkle_proof: merkle_proof.encoded_proof.clone(),
                leaf_index: merkle_proof.leaf_index,
            },
            kernel: SidechainKernel {
                version: kernel.version,
                fee: kernel.fee,
                lock_height: kernel.lock_height,
                excess: CompressedCommitment::from_canonical_bytes(kernel.excess.as_bytes())
                    .map_err(|e| anyhow!("kernel excess parse error for complete proof: {e}"))?,
                excess_sig: CompressedRistrettoSchnorr::new(
                    CompressedPublicKey::from_canonical_bytes(kernel.excess_sig.public_nonce().as_bytes())
                        .map_err(|e| anyhow!("kernel excess_sig nonce parse error for complete proof: {e}"))?,
                    RistrettoSecretKey::from_canonical_bytes(kernel.excess_sig.signature().as_bytes())
                        .map_err(|e| anyhow!("kernel excess_sig parse error for complete proof: {e}"))?,
                ),
            },
            value: proof_resp.value,
            sender_offset_public_key: CompressedPublicKey::from_canonical_bytes(sender_offset_public_key.as_bytes())
                .map_err(|e| anyhow!("sender_offset_public_key parse error for complete proof: {e}"))?,
        },
        encrypted_data: proof_resp.encrypted_data.clone(),
        mined_in_epoch: proof_resp.mined_in_epoch,
    };

    let proof = ClaimBurnProofContents {
        claim_proof: MinotariBurnClaimProof {
            burn_public_key: reciprocal_claim_public_key,
            commitment,
            ownership_proof,
            encoded_merkle_proof: EncodedMerkleProof {
                block_hash: merkle_proof
                    .block_hash
                    .as_slice()
                    .try_into()
                    .map_err(|e| anyhow!("block_hash parse error: {e}"))?,
                encoded_merkle_proof: merkle_proof
                    .encoded_proof
                    .try_into()
                    .map_err(|e| anyhow!("encoded_merkle_proof parse error: {e}"))?,
                leaf_index: merkle_proof.leaf_index,
            },
            kernel,
            value: proof_resp.value,
            sender_offset_public_key,
        },
        encrypted_data: EncryptedData::try_from(proof_resp.encrypted_data)
            .map_err(|e| anyhow!("Encrypted data length is out of bounds: {e}",))?,
    };

    world.claim_proofs.insert(proof_name, CucumberClaimProof::Confirmed {
        proof: ClaimBurnProof::Contents(Box::new(proof)),
        complete_proof: Box::new(complete_proof),
    });

    Ok(())
}

#[when(expr = "wallet {word} has at least {int} {word}")]
pub async fn check_balance(world: &mut TariWorld, step: &Step, wallet_name: String, balance: u64, units: String) {
    cucumber_log!("==== Step: {}", step.value);
    const MAX_WAIT_TIME_SECS: u64 = 100;
    let wallet = world
        .wallets
        .get(&wallet_name)
        .unwrap_or_else(|| panic!("Wallet {} not found", wallet_name));

    let mut client = wallet.create_client().await;
    let mut iterations = 0;
    let balance = match units.as_str() {
        "T" => balance * 1_000_000,
        "uT" => balance,
        _ => panic!("Unknown unit {}", units),
    };

    loop {
        let _result = client.validate_all_transactions(ValidateRequest {}).await.unwrap();
        let resp = client
            .get_balance(GetBalanceRequest { payment_id: None })
            .await
            .unwrap()
            .into_inner();
        if resp.available_balance >= balance {
            break;
        }
        cucumber_log!(
            "Waiting for wallet {} to have at least {} uT (balance: {} uT, pending: {} uT)",
            wallet_name,
            balance,
            resp.available_balance,
            resp.pending_incoming_balance
        );
        sleep(Duration::from_secs(2)).await;

        if iterations == MAX_WAIT_TIME_SECS.div_ceil(2) {
            panic!(
                "Wallet {} did not have at least {} uT after {} seconds  (balance: {} uT, pending: {} uT)",
                wallet_name, balance, MAX_WAIT_TIME_SECS, resp.available_balance, resp.pending_incoming_balance
            );
        }
        iterations += 1;
    }
}
