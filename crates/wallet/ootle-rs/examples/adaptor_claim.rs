//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! # Adaptor-signature claim of a 2-of-2 stealth output ("scriptless scripts")
//!
//! One leg of a cross-chain atomic swap, on Ootle. Funds are locked behind a script-path condition tree whose claim
//! leaf is a 2-of-2 `AllOf[P_me, P_cp]` `AccessRule` — spendable only by a transaction carrying signatures from BOTH
//! co-signers. Our side (`P_me`) we sign outright; the counterparty's side (`P_cp`) arrives as a Schnorr **adaptor
//! pre-signature** encrypted under an adaptor point `T = t·G`, which we complete with the swap secret `t`.
//!
//! Completing the signature is what makes the swap atomic: the completed, on-chain signature reveals `t` to anyone
//! holding the pre-signature (see [`adaptor_extract`]), so the counterparty can then unlock the other chain. This
//! example assumes `t` has already been obtained from the other chain (L1) — the L1 half is out of scope.
//!
//! The two one-time co-signer keys are fresh here for clarity. In production they would be derived unlinkably from the
//! output's nonce via [`ootle_rs::crypto::kdfs::spend_auth_dh_secret`] / `spend_auth_dh_public_key`, so the revealed
//! leaf exposes only one-time keys, never account identities.
//!
//! Posts two transactions on localnet (default indexer `http://127.0.0.1:12500`):
//!   1. **lock**  — faucet funds → a stealth output behind `[claim = AllOf[P_me, P_cp], refund = AfterEpoch & funder]`,
//!   2. **claim** — spends it via the script path, attaching our signature and the completed adaptor signature.
//!
//! ```bash
//! cargo run -p ootle-rs --example adaptor_claim
//! ```

use ootle_byte_type::ToByteType;
use ootle_rs::{
    Network,
    TransactionRequest,
    builtin_templates::{UnsignedTransactionBuilder, faucet::IFaucet},
    const_nonzero_u64,
    crypto::{adaptor, stealth::script_path_witness},
    default_indexer_url,
    key_provider::PrivateKeyProvider,
    provider::{PendingTransaction, Provider, ProviderBuilder, WalletProvider},
    stealth::{Output, SignatureRequirements, StealthTransfer},
    template_types::{
        UtxoAddress,
        constants::TARI_TOKEN,
        rule,
        stealth::{AtomicCondition, BuiltinPredicate, SpendCondition, StealthInput},
    },
    transaction::{
        TransactionSigner,
        adaptor::{complete_authorization, pre_sign_authorization, sign_authorization},
    },
    wallet::OotleWallet,
};
use tari_crypto::{
    keys::{PublicKey, SecretKey},
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
};
use tari_ootle_common_types::engine_types::transaction_receipt::TransactionReceipt;
use tari_ootle_transaction::{Epoch, Transaction};

/// 1 TARI expressed in microTARI.
const ONE_TARI: u64 = 1_000_000;
const LOCKED_AMOUNT: u64 = 10 * ONE_TARI;
const FEE: u64 = 2000;

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    let network = Network::LocalNet;
    let me = PrivateKeyProvider::random(network);
    let my_address = me.address().clone();
    println!("Account: {my_address}");

    let wallet = OotleWallet::from(me.clone());
    let mut provider = ProviderBuilder::new()
        .wallet(wallet)
        .connect(default_indexer_url(network))
        .await
        .unwrap();

    let current_epoch = provider.get_epoch().await.unwrap().as_u64();
    // Every transaction declares the last epoch it may be sequenced in; past it the transaction can
    // never land. Ten epochs is a comfortable window for an example — the network caps how far
    // ahead this may be set.
    let max_epoch = Epoch(provider.get_epoch().await.unwrap().as_u64() + 10);
    println!("Current epoch: {current_epoch}");

    let mut rng = rand::rng();

    // -------------------------------------------------------------------------------------------------
    // Swap secret + co-signer keys.
    // -------------------------------------------------------------------------------------------------
    // `t` is the atomic-swap secret. In a real swap it is chosen on / learned from the other chain (L1); here we
    // assume it has been obtained and it stands in for that value. `T = t·G` is the adaptor point.
    let t = RistrettoSecretKey::random(&mut rng);
    let big_t = RistrettoPublicKey::from_secret_key(&t);

    // The two one-time keys of the 2-of-2 claim leaf. `p_me` is ours; `p_cp` is the counterparty's (we hold it here
    // only to simulate their side of the protocol). See the module docs for the production (unlinkable) derivation.
    let p_me = RistrettoSecretKey::random(&mut rng);
    let pk_me = RistrettoPublicKey::from_secret_key(&p_me);
    let p_cp = RistrettoSecretKey::random(&mut rng);
    let pk_cp = RistrettoPublicKey::from_secret_key(&p_cp);

    // -------------------------------------------------------------------------------------------------
    // Condition tree: a 2-of-2 claim leaf, plus a timelocked refund to the funder.
    // -------------------------------------------------------------------------------------------------
    let deadline = current_epoch + 50;
    let claim = SpendCondition::access_rule(rule!(all_of(
        public_key(pk_me.to_byte_type()),
        public_key(pk_cp.to_byte_type())
    )));
    let refund = SpendCondition::all([
        AtomicCondition::Builtin(BuiltinPredicate::AfterEpoch(deadline)),
        AtomicCondition::AccessRule(rule!(public_key(*my_address.account_public_key()))),
    ]);
    let conditions = vec![claim, refund];
    println!("\nCondition tree ({} leaves):", conditions.len());
    println!("  claim  : AllOf[P_me, P_cp]  (2-of-2)");
    println!("  refund : All([AfterEpoch({deadline}), RequireKey(funder)])");

    let tari_token = TARI_TOKEN;

    // -------------------------------------------------------------------------------------------------
    // TX 1 — lock: faucet funds → the 2-of-2 output.
    // -------------------------------------------------------------------------------------------------
    let (lock_transfer, lock_signers) = StealthTransfer::new(tari_token, &provider)
        .spend_revealed_input(LOCKED_AMOUNT + FEE)
        .to_revealed_output(FEE)
        .to_stealth_output(
            Output::new(my_address.clone(), tari_token, const_nonzero_u64!(LOCKED_AMOUNT))
                .with_spend_conditions(conditions.clone())
                .with_memo_message("2-of-2 swap output: claim via P_me + P_cp, or refund after the deadline"),
        )
        .prepare()
        .await
        .unwrap();

    let swap_commitment = *lock_transfer.stealth_outputs()[0].commitment();

    let lock_tx = IFaucet::new(&provider, max_epoch)
        .take_faucet_funds()
        .into_stealth_transfer(lock_transfer)
        .and_pay_fee_from_revealed_output()
        .prepare()
        .await
        .expect("Failed to prepare lock transaction");

    let authorizer = provider.wallet().stealth_authorizer(lock_signers);
    let transaction = TransactionRequest::default()
        .with_transaction(lock_tx)
        .build(&authorizer)
        .await
        .unwrap();
    let pending_tx = provider.send_transaction(transaction).await.unwrap();
    print_results("Lock (faucet → 2-of-2 output)", &pending_tx).await;

    // -------------------------------------------------------------------------------------------------
    // TX 2 — claim: spend the 2-of-2 output via its script path.
    // -------------------------------------------------------------------------------------------------
    // Reveal the claim leaf (index 0) and its inclusion proof; the `AllOf` leaf takes no witness data — it is
    // satisfied by the two signer badges we attach below.
    let (claim_witness, _root) =
        script_path_witness(&conditions, &conditions[0]).expect("claim leaf is a member of the condition tree");
    let swap_input = StealthInput::with_witness(swap_commitment, claim_witness);

    let (claim_transfer, _) = StealthTransfer::new(tari_token, &provider)
        .spend_stealth_input(my_address.clone(), swap_input)
        // This claim is sealed by the account key chosen below, not by the builder's own requirements, so the key the
        // builder would infer is not the one that ends up signing. Name the receiver.
        .to_revealed_output_for(FEE, *my_address.account_public_key())
        .to_stealth_output(Output::new(
            my_address.clone(),
            tari_token,
            const_nonzero_u64!(LOCKED_AMOUNT - FEE),
        ))
        .prepare()
        .await
        .unwrap();

    let claim_tx = Transaction::builder(provider.network(), max_epoch)
        .with_fee_instructions_builder(|builder| {
            builder
                .stealth_transfer(tari_token, claim_transfer)
                .put_last_instruction_output_on_workspace("fees")
                .pay_fee_from_bucket("fees")
        })
        .add_input(tari_token)
        // The 2-of-2 UTXO being spent — DOWNed if the transaction succeeds.
        .add_input(UtxoAddress::new(tari_token, swap_commitment.into()))
        .build_unsigned();

    // Seal with the account key so the seal signer — and hence the authorization message — is fixed and known before
    // we produce the two authorizations. Both signer keys are supplied externally, so no stealth signers are derived.
    let authorizer = provider
        .wallet()
        .stealth_authorizer(SignatureRequirements::account_key_seal());
    let claim_msg = authorizer
        .authorization_message(&claim_tx)
        .await
        .expect("account-key seal yields an authorization message");

    // Our side of the 2-of-2: an ordinary signature we make outright.
    let auth_me = sign_authorization(&p_me, &claim_msg);

    // The counterparty's side arrives as an adaptor pre-signature under `T = t·G` (simulated here). We verify it can
    // only be completed with the discrete log of `T`.
    let pre = pre_sign_authorization(&p_cp, &big_t, &claim_msg);
    assert!(
        adaptor::verify(&pre, &pk_cp, &big_t, &claim_msg),
        "counterparty adaptor pre-signature must verify",
    );

    // The swap is atomic because the completed, on-chain signature reveals `t` to anyone holding the pre-signature.
    let signature_on_other_chain = adaptor::complete(&pre, &t);
    let revealed = adaptor::extract(&pre, &signature_on_other_chain).expect("a completed signature is well formed");
    assert_eq!(revealed, t, "completing the signature reveals the swap secret");
    println!("\nAdaptor signature completed; the swap secret t is now recoverable from the on-chain signature.");

    // Having obtained `t` from the other chain (L1), we complete the counterparty's leg.
    let auth_cp = complete_authorization(&pre, &revealed, &pk_cp);

    let transaction = TransactionRequest::default()
        .with_transaction(claim_tx)
        .with_authorizations([auth_me, auth_cp])
        .build(&authorizer)
        .await
        .unwrap();
    let pending_tx = provider.send_transaction(transaction).await.unwrap();
    print_results("Claim (2-of-2 via adaptor signature)", &pending_tx).await;
}

async fn print_results(label: &str, pending_tx: &PendingTransaction) -> TransactionReceipt {
    println!("\n⌛️ {label} transaction pending... {}", pending_tx.tx_id());
    let outcome = pending_tx.watch().await.unwrap();
    println!("🏁 Transaction Finalized {}", pending_tx.tx_id());
    println!("✅ Outcome: {outcome:?}");

    let receipt = pending_tx.get_receipt().await.unwrap();
    println!("-------------------------------------------");
    println!("🔹 Epoch: {}", receipt.epoch);
    println!("🔹 Transaction ID: {}", pending_tx.tx_id());
    println!("🔹 Outcome: {:?}", receipt.outcome);
    println!("🔹 Fees Paid: {}", receipt.fee_receipt.total_fees_paid());
    println!("-------------------------------------------");
    receipt
}
