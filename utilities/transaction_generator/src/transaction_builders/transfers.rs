//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use ootle_byte_type::ToByteType;
use tari_crypto::{keys::PublicKey, ristretto::RistrettoPublicKey};
use tari_ootle_common_types::{Epoch, InputDeclaration};
use tari_ootle_transaction::{Network, Transaction, args};
use tari_template_lib_types::{Amount, VaultId, constants::TARI_TOKEN};

use super::accounts::{account_address, account_keypair};

/// What each transfer moves. Small enough that a faucet-funded account pays for every flood it will see.
const TRANSFER_AMOUNT: Amount = Amount::new(1_000);

/// Transfers out of the accounts derived from `seed`, transaction `i` spending from account `i` into a new
/// account for a fresh key.
///
/// No two transactions write the same substate: each writes only its own account and that account's TARI vault,
/// and reads the TARI resource. `vaults[i]` must be account `i`'s TARI vault.
pub fn builder(network: Network, max_epoch: Epoch, seed: u64, vaults: Vec<VaultId>) -> impl Fn(u64) -> Transaction {
    move |index: u64| -> Transaction {
        let (signer_secret_key, signer_public_key) = account_keypair(seed, index);
        let account = account_address(&signer_public_key);
        let vault = vaults[usize::try_from(index).expect("transaction index fits in usize")];
        let (_, recipient) = RistrettoPublicKey::random_keypair(&mut rand::rng());

        Transaction::builder(network.as_byte(), max_epoch)
            // Allocated well above what a transfer into a new account costs; the excess refunds.
            .pay_fee_from_component(account, Amount::new(50_000))
            .call_method(account, "withdraw", args![TARI_TOKEN, TRANSFER_AMOUNT])
            .put_last_instruction_output_on_workspace("bucket")
            .create_account_with_bucket(recipient.to_byte_type(), "bucket")
            .with_inputs([
                InputDeclaration::write(account),
                InputDeclaration::write(vault),
                InputDeclaration::read(TARI_TOKEN),
            ])
            .build_and_seal(&signer_secret_key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use tari_template_lib_types::ObjectKey;

    use super::*;

    #[test]
    fn transfers_write_disjoint_substates() {
        let vaults = (0..3u8)
            .map(|i| VaultId::new(ObjectKey::from_array([i; ObjectKey::LENGTH])))
            .collect();
        let build = builder(Network::LocalNet, Epoch(1), 42, vaults);

        let mut written = HashSet::new();
        for index in 0..3 {
            for input in build(index).inputs() {
                if input.is_write() {
                    assert!(
                        written.insert(input.substate_id().clone()),
                        "{} is written by more than one transfer",
                        input.substate_id()
                    );
                }
            }
        }
        assert_eq!(written.len(), 6);
    }
}
