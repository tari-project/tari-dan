//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

/// These would live in the wallet
use rand::rngs::OsRng;
use tari_common_types::types::{BulletRangeProof, PrivateKey, PublicKey, Signature};
use tari_crypto::{
    commitment::{ExtensionDegree, HomomorphicCommitmentFactory},
    errors::RangeProofError,
    extended_range_proof::ExtendedRangeProofService,
    keys::{PublicKey as _, SecretKey},
    ristretto::bulletproofs_plus::{RistrettoExtendedMask, RistrettoExtendedWitness},
    tari_utilities::ByteArray,
};
use tari_engine_types::confidential::{challenges, get_commitment_factory, get_range_proof_service};
use tari_template_lib::{
    crypto::{BalanceProofSignature, RistrettoPublicKeyBytes},
    models::{Amount, ConfidentialOutputProof, ConfidentialStatement, ConfidentialWithdrawProof, EncryptedData},
};

pub struct ConfidentialProofStatement {
    pub amount: Amount,
    pub mask: PrivateKey,
    pub sender_public_nonce: PublicKey,
    pub minimum_value_promise: u64,
}

pub fn generate_confidential_proof(
    output_amount: Amount,
    change: Option<Amount>,
) -> (ConfidentialOutputProof, PrivateKey, Option<PrivateKey>) {
    let mask = PrivateKey::random(&mut OsRng);
    let output_statement = ConfidentialProofStatement {
        amount: output_amount,
        mask: mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
    };

    let change_mask = PrivateKey::random(&mut OsRng);
    let change_statement = change.map(|amount| ConfidentialProofStatement {
        amount,
        mask: change_mask.clone(),
        sender_public_nonce: Default::default(),
        minimum_value_promise: 0,
    });

    let proof = generate_confidential_proof_from_statements(output_statement, change_statement).unwrap();
    (proof, mask, change.map(|_| change_mask))
}

pub fn generate_balance_proof(
    input_mask: &PrivateKey,
    output_mask: &PrivateKey,
    change_mask: Option<&PrivateKey>,
    revealed_amount: Amount,
) -> BalanceProofSignature {
    let secret_excess = input_mask - output_mask - change_mask.unwrap_or(&PrivateKey::default());
    let excess = PublicKey::from_secret_key(&secret_excess);
    let (nonce, public_nonce) = PublicKey::random_keypair(&mut OsRng);
    let challenge = challenges::confidential_withdraw64(&excess, &public_nonce, revealed_amount);

    let sig = Signature::sign_raw_uniform(&secret_excess, nonce, &challenge).unwrap();
    BalanceProofSignature::try_from_parts(sig.get_public_nonce().as_bytes(), sig.get_signature().as_bytes()).unwrap()
}

pub struct WithdrawProofOutput {
    pub output_mask: PrivateKey,
    pub change_mask: Option<PrivateKey>,
    pub proof: ConfidentialWithdrawProof,
}

pub fn generate_withdraw_proof(
    input_mask: &PrivateKey,
    output_amount: Amount,
    change_amount: Option<Amount>,
    revealed_amount: Amount,
) -> WithdrawProofOutput {
    let (output_proof, output_mask, change_mask) = generate_confidential_proof(output_amount, change_amount);
    let total_amount = output_amount + change_amount.unwrap_or_else(Amount::zero) + revealed_amount;
    let input_commitment = get_commitment_factory().commit_value(input_mask, total_amount.value() as u64);
    let input_commitment = copy_fixed(input_commitment.as_bytes());
    let balance_proof = generate_balance_proof(input_mask, &output_mask, change_mask.as_ref(), revealed_amount);

    let output_statement = output_proof.output_statement;

    WithdrawProofOutput {
        output_mask,
        change_mask,
        proof: ConfidentialWithdrawProof {
            inputs: vec![input_commitment],
            output_proof: ConfidentialOutputProof {
                output_statement: ConfidentialStatement {
                    commitment: output_statement.commitment,
                    sender_public_nonce: Default::default(),
                    encrypted_data: EncryptedData::default(),
                    minimum_value_promise: output_statement.minimum_value_promise,
                    revealed_amount,
                },
                change_statement: output_proof.change_statement.map(|statement| ConfidentialStatement {
                    commitment: statement.commitment,
                    sender_public_nonce: Default::default(),
                    encrypted_data: EncryptedData::default(),
                    minimum_value_promise: statement.minimum_value_promise,
                    revealed_amount: Amount::zero(),
                }),
                range_proof: output_proof.range_proof,
            },
            balance_proof,
        },
    }
}

pub fn generate_withdraw_proof_with_inputs(
    input: &[(PrivateKey, Amount)],
    output_amount: Amount,
    change_amount: Option<Amount>,
    revealed_amount: Amount,
) -> WithdrawProofOutput {
    let (output_proof, output_mask, change_mask) = generate_confidential_proof(output_amount, change_amount);
    let input_commitments = input
        .iter()
        .map(|(input_mask, amount)| {
            let input_commitment = get_commitment_factory().commit_value(input_mask, amount.value() as u64);
            copy_fixed(input_commitment.as_bytes())
        })
        .collect();
    let input_private_excess = input
        .iter()
        .fold(PrivateKey::default(), |acc, (input_mask, _)| acc + input_mask);
    let balance_proof = generate_balance_proof(
        &input_private_excess,
        &output_mask,
        change_mask.as_ref(),
        revealed_amount,
    );

    let output_statement = output_proof.output_statement;
    let change_statement = output_proof.change_statement;

    WithdrawProofOutput {
        output_mask,
        change_mask,
        proof: ConfidentialWithdrawProof {
            inputs: input_commitments,
            output_proof: ConfidentialOutputProof {
                output_statement: ConfidentialStatement {
                    commitment: output_statement.commitment,
                    // R and encrypted value are informational and can be left out as far as the VN is concerned
                    sender_public_nonce: Default::default(),
                    encrypted_data: Default::default(),
                    minimum_value_promise: output_statement.minimum_value_promise,
                    revealed_amount,
                },
                change_statement: change_statement.map(|change| ConfidentialStatement {
                    commitment: change.commitment,
                    sender_public_nonce: Default::default(),
                    encrypted_data: Default::default(),
                    minimum_value_promise: change.minimum_value_promise,
                    revealed_amount: Amount::zero(),
                }),
                range_proof: output_proof.range_proof,
            },
            balance_proof,
        },
    }
}

fn copy_fixed<const SZ: usize>(bytes: &[u8]) -> [u8; SZ] {
    let mut array = [0u8; SZ];
    array.copy_from_slice(&bytes[..SZ]);
    array
}

fn generate_confidential_proof_from_statements(
    output_statement: ConfidentialProofStatement,
    change_statement: Option<ConfidentialProofStatement>,
) -> Result<ConfidentialOutputProof, RangeProofError> {
    let proof_change_statement = change_statement.as_ref().map(|statement| ConfidentialStatement {
        commitment: commitment_to_bytes(&statement.mask, statement.amount),
        sender_public_nonce: RistrettoPublicKeyBytes::from_bytes(statement.sender_public_nonce.as_bytes())
            .expect("[generate_confidential_proof] change nonce"),
        encrypted_data: Default::default(),
        minimum_value_promise: statement.minimum_value_promise,
        revealed_amount: Amount::zero(),
    });

    let output_range_proof = generate_extended_bullet_proof(&output_statement, change_statement.as_ref())?;

    Ok(ConfidentialOutputProof {
        output_statement: ConfidentialStatement {
            commitment: commitment_to_bytes(&output_statement.mask, output_statement.amount),
            sender_public_nonce: RistrettoPublicKeyBytes::from_bytes(output_statement.sender_public_nonce.as_bytes())
                .expect("[generate_confidential_proof] output nonce"),
            encrypted_data: Default::default(),
            minimum_value_promise: output_statement.minimum_value_promise,
            revealed_amount: Amount::zero(),
        },
        change_statement: proof_change_statement,
        range_proof: output_range_proof.0,
    })
}

fn generate_extended_bullet_proof(
    output_statement: &ConfidentialProofStatement,
    change_statement: Option<&ConfidentialProofStatement>,
) -> Result<BulletRangeProof, RangeProofError> {
    let mut extended_witnesses = vec![];

    let extended_mask =
        RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![output_statement.mask.clone()]).unwrap();

    let mut agg_factor = 1;
    extended_witnesses.push(RistrettoExtendedWitness {
        mask: extended_mask,
        value: output_statement.amount.value() as u64,
        minimum_value_promise: output_statement.minimum_value_promise,
    });
    if let Some(stmt) = change_statement {
        let extended_mask =
            RistrettoExtendedMask::assign(ExtensionDegree::DefaultPedersen, vec![stmt.mask.clone()]).unwrap();
        extended_witnesses.push(RistrettoExtendedWitness {
            mask: extended_mask,
            value: stmt.amount.value() as u64,
            minimum_value_promise: stmt.minimum_value_promise,
        });
        agg_factor = 2;
    }

    let output_range_proof = get_range_proof_service(agg_factor).construct_extended_proof(extended_witnesses, None)?;
    Ok(BulletRangeProof(output_range_proof))
}

fn commitment_to_bytes(mask: &PrivateKey, amount: Amount) -> [u8; 32] {
    let commitment = get_commitment_factory().commit_value(mask, amount.value() as u64);
    let mut bytes = [0u8; 32];
    bytes.copy_from_slice(commitment.as_bytes());
    bytes
}
