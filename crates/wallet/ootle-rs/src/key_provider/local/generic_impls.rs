//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::mem;

use async_trait::async_trait;
use ootle_byte_type::{FromByteType, ToByteType};
use signature::hazmat::PrehashSigner;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_address::RistrettoOotleAddress;
use tari_ootle_common_types::signature::Sr25519SignatureBytes;
use tari_ootle_transaction::{
    Signable,
    TransactionSealSignature,
    TransactionSignature,
    UnsealedTransaction,
    UnsignedTransaction,
};
use tari_ootle_wallet_crypto::{
    OutputWitness,
    StealthCryptoApi,
    StealthOutputWitness,
    balance_proof::{generate_stealth_balance_proof_signature, validate_balance_proof_signature},
    bullet_proof::generate_extended_bullet_proof,
    stealth::pay_to_output_authorization,
    viewable_balance_proof::generate_elgamal_viewable_balance_proof,
};
use tari_template_lib_types::{
    EncryptedData,
    crypto::RistrettoPublicKeyBytes,
    stealth::{
        RevealedOutput,
        StealthInputsStatement,
        StealthOutputsStatement,
        StealthTransferStatement,
        StealthUnspentOutput,
        UnspentOutput,
    },
};
use tokio::task;

use crate::{
    Address,
    key_provider::{LocalKeyProvider, OutputMaskProvider},
    signer,
    signer::StealthKeyPrehashSigner,
    stealth::{
        InputDecryptor,
        Output,
        ResolvedStealthTransferSpec,
        StealthOutputStatementFactory,
        StealthProviderError,
        StealthResult,
        StealthStatementProvider,
    },
    transaction::{TransactionSigner, TransactionStealthKeySigner},
    wallet::TransactionAuthorization,
};

#[async_trait]
impl<C> TransactionSigner for LocalKeyProvider<C>
where C: PrehashSigner<(Sr25519SignatureBytes, RistrettoPublicKey)> + Send + Sync
{
    fn address(&self) -> &Address {
        &self.address
    }

    async fn sign_transaction(&self, message: &UnsealedTransaction) -> signer::Result<TransactionSealSignature> {
        let message = message.to_signing_message(());
        let (signature, public_key) = self.credentials.sign_prehash(&message)?;
        let sig = TransactionSealSignature::new(public_key.to_byte_type(), signature);
        Ok(sig)
    }

    async fn sign_authorization(
        &self,
        seal_signer: &RistrettoPublicKeyBytes,
        tx: &UnsignedTransaction,
    ) -> signer::Result<TransactionAuthorization> {
        let message = tx.to_signing_message(seal_signer);
        let (signature, public_key) = self.credentials.sign_prehash(&message)?;
        let sig = TransactionSignature::new(public_key.to_byte_type(), signature);
        Ok(sig.into())
    }
}

#[async_trait]
impl<C: OutputMaskProvider + Send + Sync> StealthOutputStatementFactory for LocalKeyProvider<C> {
    async fn generate_outputs_statement(
        &self,
        specs: Vec<Output>,
        revealed_output: Option<RevealedOutput>,
    ) -> StealthResult<(StealthOutputsStatement, RistrettoSecretKey)> {
        let mut outputs = Vec::with_capacity(specs.len());
        let mut witnesses = Vec::with_capacity(specs.len());
        let mut agg_output_mask = RistrettoSecretKey::default();
        for spec in specs {
            let StealthOutputWitness { mut witness, auth, tag } =
                create_output_witness(&self.credentials, spec).await?;

            let commitment = witness.to_commitment();
            agg_output_mask = &agg_output_mask + &witness.mask;

            outputs.push(StealthUnspentOutput {
                output: UnspentOutput {
                    commitment: commitment.to_byte_type(),
                    sender_public_nonce: witness.sender_public_nonce.to_byte_type(),
                    minimum_value_promise: witness.minimum_value_promise,
                    viewable_balance_proof: witness
                        .resource_view_key
                        .as_ref()
                        .map(|pk| {
                            generate_elgamal_viewable_balance_proof(&witness.mask, witness.amount, &commitment, pk)
                        })
                        .transpose()?,
                    // Move the encrypted data out of the witness, we don't need it in the bullet proof generation
                    encrypted_data: mem::replace(&mut witness.encrypted_data, EncryptedData::empty()),
                },
                auth,
                tag,
            });

            witnesses.push(witness);
        }

        let agg_range_proof = task::spawn_blocking(move || generate_extended_bullet_proof(&witnesses))
            .await
            .map_err(|e| StealthProviderError::SpawnBlockingPanic { details: e.to_string() })?
            .map_err(|e| StealthProviderError::RangeProofError { details: e.to_string() })?;

        Ok((
            StealthOutputsStatement {
                outputs,
                revealed_output,
                agg_range_proof,
            },
            agg_output_mask,
        ))
    }
}

#[async_trait]
impl<C> StealthStatementProvider for LocalKeyProvider<C>
where LocalKeyProvider<C>: StealthOutputStatementFactory + InputDecryptor + Send + Sync
{
    async fn create_transfer_statement(
        &self,
        spec: ResolvedStealthTransferSpec,
    ) -> StealthResult<StealthTransferStatement> {
        let total_output_amount = spec.total_output_amount();
        let total_revealed_input = spec.revealed_input_amount;
        let requires_balance_proof = spec.requires_balance_proof();

        let ResolvedStealthTransferSpec {
            inputs,
            revealed_input_amount,
            outputs,
            revealed_output,
        } = spec;

        let mut agg_input_mask = RistrettoSecretKey::default();
        let mut statement_inputs = Vec::with_capacity(inputs.len());
        for resolved in inputs {
            let decrypted = self
                .decrypt_input_data(resolved.commitment(), &resolved.output, true)
                .await?;
            agg_input_mask = &agg_input_mask + decrypted.mask();
            statement_inputs.push(resolved.input);
        }

        let (outputs_statement, agg_output_mask) = self.generate_outputs_statement(outputs, revealed_output).await?;

        let inputs_statement = StealthInputsStatement {
            inputs: statement_inputs,
            revealed_amount: revealed_input_amount,
        };

        // This provider spends only key-path inputs, so no partition gates on a covenant and there is nothing to
        // claim. The claims are bound by the balance proof, so they must be settled before it is signed.
        let covenant_claims = vec![];

        let balance_proof = requires_balance_proof.then(|| {
            generate_stealth_balance_proof_signature(
                &agg_input_mask,
                &agg_output_mask,
                &inputs_statement,
                &outputs_statement,
                &covenant_claims,
            )
        });

        if let Some(balance_proof) = &balance_proof {
            // Every proof above is generated from our own key material, so a balance proof that does not
            // verify means the caller's input and output values do not balance.
            if !validate_balance_proof_signature(balance_proof, &inputs_statement, &outputs_statement, &covenant_claims)
            {
                return Err(StealthProviderError::UnbalancedTransfer {
                    total_revealed_input,
                    output_amount: total_output_amount,
                });
            }
        }

        Ok(StealthTransferStatement {
            inputs_statement,
            outputs_statement,
            balance_proof,
            covenant_claims: Vec::new(),
        })
    }
}

async fn create_output_witness<K: OutputMaskProvider>(
    key_provider: &K,
    spec: Output,
) -> Result<StealthOutputWitness, StealthProviderError> {
    let mask = key_provider
        .next_mask()
        .await
        .map_err(|e| StealthProviderError::UnexpectedError { details: e.to_string() })?;
    let Output {
        destination,
        amount,
        resource_address,
        resource_view_key,
        memo,
        pay_to,
        ..
    } = spec;

    let destination: RistrettoOotleAddress =
        destination
            .try_from_byte_type()
            .map_err(|_| StealthProviderError::InvalidDestinationAddress {
                details: format!("{destination} is not a valid RistrettoOotleAddress"),
            })?;

    let crypto_api = StealthCryptoApi::new();

    let (nonce_secret, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());
    let encrypted_data = crypto_api.encrypt_value_and_mask(
        amount.get(),
        &mask,
        destination.view_only_key(),
        &nonce_secret,
        memo.as_ref(),
    )?;

    let auth = pay_to_output_authorization(&pay_to, || {
        // Create stealth address that the destination can use at spend time
        crypto_api
            .derive_stealth_owner_public_key(
                destination.network(),
                destination.account_key(),
                destination.view_only_key(),
                &nonce_secret,
            )
            .to_byte_type()
    })
    .map_err(|e| StealthProviderError::UnexpectedError { details: e.to_string() })?;

    let witness = OutputWitness {
        amount: amount.get(),
        mask,
        sender_public_nonce: public_nonce,
        encrypted_data,
        minimum_value_promise: spec.minimum_value_promise,
        resource_view_key,
    };

    let derived_tag = spec.utxo_tag.unwrap_or_else(|| {
        crypto_api.derive_stealth_output_tag(
            destination.network(),
            &nonce_secret,
            destination.view_only_key(),
            &resource_address,
        )
    });

    Ok(StealthOutputWitness {
        witness,
        auth,
        tag: derived_tag,
    })
}

#[async_trait]
impl<C: StealthKeyPrehashSigner<(Sr25519SignatureBytes, RistrettoPublicKey)> + Send + Sync> TransactionStealthKeySigner
    for LocalKeyProvider<C>
{
    async fn sign_authorization_with_stealth(
        &self,
        public_nonce: &RistrettoPublicKey,
        seal_signer: &RistrettoPublicKeyBytes,
        tx: &UnsignedTransaction,
    ) -> signer::Result<TransactionAuthorization> {
        let (sig, pk) = self
            .credentials
            .sign_prehash_with_stealth_key(public_nonce, &tx.to_signing_message(seal_signer))
            .await?;
        let sig = TransactionSignature::new(pk.to_byte_type(), sig);
        Ok(sig.into())
    }

    async fn seal_transaction_with_stealth(
        &self,
        public_nonce: &RistrettoPublicKey,
        message: &UnsealedTransaction,
    ) -> signer::Result<TransactionSealSignature> {
        let message = message.to_signing_message(());
        let (signature, public_key) = self
            .credentials
            .sign_prehash_with_stealth_key(public_nonce, &message)
            .await?;
        let sig = TransactionSealSignature::new(public_key.to_byte_type(), signature);
        Ok(sig)
    }

    async fn stealth_public_key(&self, public_nonce: &RistrettoPublicKey) -> signer::Result<RistrettoPublicKeyBytes> {
        let public_key = self.credentials.stealth_public_key(public_nonce).await?;
        Ok(public_key.to_byte_type())
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZeroU64;

    use tari_crypto::keys::SecretKey;
    use tari_ootle_common_types::engine_types::{crypto::OutputBody, stealth::validate_transfer};
    use tari_ootle_wallet_crypto::MaskAndValue;
    use tari_template_lib_types::{Amount, ResourceAddress, constants::TARI_TOKEN, stealth::StealthInput};

    use super::*;
    use crate::{
        Network,
        key_provider::PrivateKeyProvider,
        stealth::{BurnClaimKeyProvider, BurnClaimStatementSpec, ResolvedStealthInput},
    };

    const VALUE: u64 = 1_000_000;

    /// A stand-in receiver for a revealed output. These tests exercise statement construction, not the engine's
    /// badge check, so any canonical key will do.
    fn a_receiver() -> RistrettoPublicKeyBytes {
        RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(9u64)).to_byte_type()
    }

    fn resource() -> ResourceAddress {
        TARI_TOKEN
    }

    /// Mint a stealth output owned by `provider` and return it shaped as an input ready to spend.
    ///
    /// The output is encrypted to the destination's view-only key, so a provider spending its own
    /// output recovers the same mask and value the mint committed to.
    async fn owned_input(provider: &PrivateKeyProvider, value: u64) -> ResolvedStealthInput {
        let spec = Output::new(
            provider.address().clone(),
            resource(),
            NonZeroU64::new(value).expect("test value is non-zero"),
        );
        let (statement, _agg_mask) = provider
            .generate_outputs_statement(vec![spec], None)
            .await
            .expect("minting a stealth output must succeed");

        let minted = statement.outputs.into_iter().next().expect("one output was requested");
        ResolvedStealthInput::new(StealthInput::from(minted.output.commitment), OutputBody {
            public_nonce: minted.output.sender_public_nonce,
            encrypted_data: minted.output.encrypted_data,
            minimum_value_promise: minted.output.minimum_value_promise,
            viewable_balance: None,
        })
    }

    fn output_to_self(provider: &PrivateKeyProvider, value: u64) -> Output {
        Output::new(
            provider.address().clone(),
            resource(),
            NonZeroU64::new(value).expect("test value is non-zero"),
        )
    }

    /// A stealth input spent into an output of equal value produces a statement the engine accepts.
    #[tokio::test]
    async fn spending_a_stealth_input_produces_a_valid_statement() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let input = owned_input(&provider, VALUE).await;

        let statement = provider
            .create_transfer_statement(ResolvedStealthTransferSpec {
                inputs: vec![input],
                revealed_input_amount: Amount::zero(),
                outputs: vec![output_to_self(&provider, VALUE)],
                revealed_output: None,
            })
            .await
            .expect("a balanced transfer must produce a statement");

        assert!(statement.balance_proof.is_some());
        assert_eq!(statement.inputs_statement.inputs.len(), 1);
        assert_eq!(statement.outputs_statement.outputs.len(), 1);
        validate_transfer(&statement, None).expect("the engine must accept the statement");
    }

    /// Several inputs and outputs balance in aggregate, and the input order is preserved.
    #[tokio::test]
    async fn multiple_inputs_and_outputs_balance_in_aggregate() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let inputs = vec![
            owned_input(&provider, VALUE).await,
            owned_input(&provider, VALUE * 2).await,
        ];
        let expected_commitments: Vec<_> = inputs.iter().map(|i| *i.commitment()).collect();

        let statement = provider
            .create_transfer_statement(ResolvedStealthTransferSpec {
                inputs,
                revealed_input_amount: Amount::zero(),
                outputs: vec![output_to_self(&provider, VALUE), output_to_self(&provider, VALUE * 2)],
                revealed_output: None,
            })
            .await
            .expect("a balanced transfer must produce a statement");

        let actual_commitments: Vec<_> = statement.inputs_statement.inputs.iter().map(|i| i.commitment).collect();
        assert_eq!(actual_commitments, expected_commitments);
        validate_transfer(&statement, None).expect("the engine must accept the statement");
    }

    /// A revealed input covers a stealth output of the same value.
    #[tokio::test]
    async fn revealed_input_funding_a_stealth_output_validates() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);

        let statement = provider
            .create_transfer_statement(ResolvedStealthTransferSpec {
                inputs: vec![],
                revealed_input_amount: Amount::from(VALUE),
                outputs: vec![output_to_self(&provider, VALUE)],
                revealed_output: None,
            })
            .await
            .expect("a balanced transfer must produce a statement");

        assert!(statement.balance_proof.is_some());
        validate_transfer(&statement, None).expect("the engine must accept the statement");
    }

    /// Inputs that do not cover the outputs are rejected rather than yielding a statement the engine
    /// would later refuse.
    #[tokio::test]
    async fn an_unbalanced_transfer_is_rejected() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let input = owned_input(&provider, VALUE).await;

        let err = provider
            .create_transfer_statement(ResolvedStealthTransferSpec {
                inputs: vec![input],
                revealed_input_amount: Amount::zero(),
                // Spend more than the input holds.
                outputs: vec![output_to_self(&provider, VALUE + 1)],
                revealed_output: None,
            })
            .await
            .expect_err("an unbalanced transfer must be rejected");

        assert!(
            matches!(err, StealthProviderError::UnbalancedTransfer { .. }),
            "expected UnbalancedTransfer, got {err:?}"
        );
    }

    /// A transfer that moves only revealed value has nothing to balance, so it carries no balance
    /// proof.
    #[tokio::test]
    async fn a_revealed_only_transfer_has_no_balance_proof() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);

        let statement = provider
            .create_transfer_statement(ResolvedStealthTransferSpec {
                inputs: vec![],
                revealed_input_amount: Amount::from(VALUE),
                outputs: vec![],
                revealed_output: Some(RevealedOutput::new(Amount::from(VALUE), a_receiver())),
            })
            .await
            .expect("a revealed-only transfer must produce a statement");

        assert!(statement.balance_proof.is_none());
    }

    /// A statement built by one wallet does not validate against another wallet's input: the mask
    /// recovered from a foreign output is not the one the commitment was built with.
    #[tokio::test]
    async fn an_input_owned_by_another_wallet_does_not_balance() {
        let alice = PrivateKeyProvider::random(Network::LocalNet);
        let bob = PrivateKeyProvider::random(Network::LocalNet);
        let alices_input = owned_input(&alice, VALUE).await;

        let err = bob
            .create_transfer_statement(ResolvedStealthTransferSpec {
                inputs: vec![alices_input],
                revealed_input_amount: Amount::zero(),
                outputs: vec![output_to_self(&bob, VALUE)],
                revealed_output: None,
            })
            .await
            .expect_err("bob must not be able to spend alice's output");

        assert!(
            matches!(
                err,
                StealthProviderError::UnbalancedTransfer { .. } | StealthProviderError::DecryptionFailed { .. }
            ),
            "expected the spend to fail, got {err:?}"
        );
    }

    #[test]
    fn total_output_amount_sums_stealth_and_revealed_outputs() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let spec = ResolvedStealthTransferSpec {
            inputs: vec![],
            revealed_input_amount: Amount::zero(),
            outputs: vec![output_to_self(&provider, 300), output_to_self(&provider, 700)],
            revealed_output: Some(RevealedOutput::new(Amount::from(1000u128), a_receiver())),
        };
        assert_eq!(spec.total_output_amount(), Amount::from(2000u128));
    }

    #[test]
    fn a_transfer_needs_a_balance_proof_only_when_stealth_value_moves() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let revealed_only = ResolvedStealthTransferSpec {
            inputs: vec![],
            revealed_input_amount: Amount::from(VALUE),
            outputs: vec![],
            revealed_output: Some(RevealedOutput::new(Amount::from(VALUE), a_receiver())),
        };
        assert!(!revealed_only.requires_balance_proof());

        let with_stealth_output = ResolvedStealthTransferSpec {
            outputs: vec![output_to_self(&provider, VALUE)],
            ..revealed_only
        };
        assert!(with_stealth_output.requires_balance_proof());
    }

    /// A burn claim spends the minted burn UTXO into a stealth output plus a revealed fee, and the
    /// engine accepts the resulting statement.
    ///
    /// The L1 burn output is encrypted to the claimant's *account* key (not its view-only key) with the
    /// burn's sender-offset nonce, which is the shape `decrypt_burn_claim_output` expects.
    #[tokio::test]
    async fn a_burn_claim_statement_balances_and_validates() {
        const FEE: u64 = 1000;

        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let account_pk = RistrettoPublicKey::from_secret_key(provider.credentials().account_secret());

        // Stand in for the L1 burn: a commitment to `VALUE` under `mask`, encrypted to the claimant.
        let (sender_offset_secret, sender_offset_public_key) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let mask = RistrettoSecretKey::random(&mut rand::rng());
        let commitment = MaskAndValue::new(VALUE, mask.clone()).to_commitment().to_byte_type();
        let encrypted_data = StealthCryptoApi::new()
            .encrypt_value_and_mask(VALUE, &mask, &account_pk, &sender_offset_secret, None)
            .expect("encrypting the burn output must succeed");

        let statement = provider
            .create_burn_claim_statement(BurnClaimStatementSpec {
                commitment,
                encrypted_data,
                sender_offset_public_key,
                output: output_to_self(&provider, VALUE - FEE),
                revealed_output: Some(RevealedOutput::new(Amount::from(u128::from(FEE)), a_receiver())),
            })
            .await
            .expect("a balanced burn claim must produce a statement");

        assert!(statement.balance_proof.is_some());
        assert_eq!(statement.inputs_statement.inputs.len(), 1);
        assert_eq!(statement.inputs_statement.inputs[0].commitment, commitment);
        validate_transfer(&statement, None).expect("the engine must accept the burn claim statement");
    }

    /// `stealth_public_key` must agree with the key the signing methods actually use, since authorization signatures
    /// commit to it before any signature over the transaction is made.
    #[tokio::test]
    async fn the_derived_stealth_public_key_is_the_one_that_signs() {
        let provider = PrivateKeyProvider::random(Network::LocalNet);
        let (_, public_nonce) = RistrettoPublicKey::random_keypair(&mut rand::rng());
        let unsigned =
            tari_ootle_transaction::Transaction::builder(Network::LocalNet, tari_ootle_transaction::Epoch(1))
                .build_unsigned();

        let derived = provider
            .stealth_public_key(&public_nonce)
            .await
            .expect("a local provider derives its own stealth key");

        let seal = provider
            .seal_transaction_with_stealth(&public_nonce, &unsigned.clone().finish())
            .await
            .expect("sealing with a stealth key must succeed");
        assert_eq!(seal.public_key(), &derived);

        let auth = provider
            .sign_authorization_with_stealth(&public_nonce, &derived, &unsigned)
            .await
            .expect("authorizing with a stealth key must succeed");
        assert_eq!(auth.signature().public_key(), &derived);
    }
}
