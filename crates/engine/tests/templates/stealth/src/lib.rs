//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::prelude::*;

#[template]
mod template {
    use tari_template_lib::prelude::crypto::CommitmentValueProof;

    use super::*;

    pub struct StealthFaucet {
        manager: ResourceManager,
        supply_vault: Vault,
    }

    impl StealthFaucet {
        pub fn new(
            initial_supply: Amount,
            mint: StealthTransferStatement,
            view_key: Option<RistrettoPublicKeyBytes>,
        ) -> Component<Self> {
            let signer = CallerContext::transaction_signer_public_key();
            let bucket = ResourceBuilder::stealth()
                .mintable(rule!(allow_all), OWNER)
                .freezable(rule!(public_key(signer)), OWNER)
                .with_view_key_opt(view_key)
                .initial_supply(initial_supply);

            let resource_address = bucket.resource_address();
            // Convert the minted funds to UTXOs as per the stealth transfer.
            let revealed_output_bucket = bucket.stealth_transfer(mint);
            let supply_vault = Vault::from_bucket(revealed_output_bucket);

            Component::new(Self {
                manager: resource_address.into(),
                supply_vault,
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        /// Like [`new`], but the resource's withdraw rule requires the creating signer's badge. Every
        /// stealth transfer of the resource must then be authorised by that key, exercising the
        /// resource-level withdraw gate on the stealth-transfer path. The in-`new` mint is itself a
        /// stealth transfer, so it only succeeds because this construction is signed by that same key.
        pub fn new_withdraw_gated_by_signer(initial_supply: Amount, mint: StealthTransferStatement) -> Component<Self> {
            let signer = CallerContext::transaction_signer_public_key();
            let bucket = ResourceBuilder::stealth()
                .mintable(rule!(allow_all), OWNER)
                .withdrawable(rule!(public_key(signer)), OWNER)
                .initial_supply(initial_supply);

            let resource_address = bucket.resource_address();
            let revealed_output_bucket = bucket.stealth_transfer(mint);
            let supply_vault = Vault::from_bucket(revealed_output_bucket);

            Component::new(Self {
                manager: resource_address.into(),
                supply_vault,
            })
            .with_access_rules(AccessRules::allow_all())
            .create()
        }

        /// Returns the bucket alongside a proof that locks its funds, so a caller can hand a partially locked
        /// bucket to an instruction that consumes buckets whole.
        pub fn lock_bucket(bucket: Bucket) -> (Bucket, Proof) {
            let proof = bucket.create_proof();
            (bucket, proof)
        }

        pub fn take_funds(&self, amount: Amount) -> Bucket {
            self.supply_vault.withdraw(amount)
        }

        pub fn programmatic_transfer(&self, transfer: StealthTransferStatement) {
            // You could check the output revealed amount before calling stealth transfer - however, this is not
            // strictly necessary because the transfer below will fail (returned bucket will be None) if the
            // revealed output amount is zero.
            //
            // if transfer.outputs_statement.revealed_output_amount <= Amount::zero() {
            //     panic!("Revealed output amount must be positive");
            // }

            // If there are any revealed inputs required, we'll take it from the supply vault.
            let maybe_input_bucket = if transfer.inputs_statement.revealed_amount.is_positive() {
                Some(self.supply_vault.withdraw(transfer.inputs_statement.revealed_amount))
            } else {
                None
            };

            let bucket = self
                .manager
                .stealth_transfer_with_opt_input_bucket(transfer, maybe_input_bucket)
                .expect("Stealth transfers must revealed output amounts (which we'll take for ourselves mwahaha!)");
            // All revealed funds are transferred to the component's vault.
            self.supply_vault.deposit(bucket);
        }

        pub fn static_programmatic_transfer(resource: ResourceAddress, transfer: StealthTransferStatement) {
            let manager = ResourceManager::get(resource);
            manager.stealth_transfer(transfer);
        }

        /// Burns `rounds` of metered compute (a serial dependent division chain, mirroring the
        /// `metering_bench` template) and then performs a programmatic stealth transfer, all inside
        /// one WASM invocation. Exercises a native-verification charge issued mid-invocation, after
        /// real in-flight metering consumption.
        pub fn burn_compute_then_transfer(&self, rounds: u64, transfer: StealthTransferStatement) -> u64 {
            // Returned so the optimiser cannot eliminate the pure grind loop.
            let acc = Self::grind(rounds);
            self.programmatic_transfer(transfer);
            acc
        }

        /// The compute burner alone, for calibrating points-per-round in tests.
        pub fn burn_compute(&self, rounds: u64) -> u64 {
            Self::grind(rounds)
        }

        fn grind(rounds: u64) -> u64 {
            let mut acc: u64 = 0xD1B5_4A32_D192_ED03;
            let mut i: u64 = 0;
            while i < rounds {
                let mut j: u32 = 0;
                while j < 64 {
                    let d = (acc & 0xFFFF) | 1;
                    let v = acc.wrapping_div(d);
                    acc = (v ^ d).rotate_left(1).wrapping_mul(0x9E37_79B9_7F4A_7C17);
                    j = j.wrapping_add(1);
                }
                i = i.wrapping_add(1);
            }
            acc
        }

        pub fn mint(&self, amount: Amount) {
            let bucket = self.manager.mint_stealth(amount);
            self.supply_vault.deposit(bucket);
        }

        pub fn freeze_utxos(&self, utxos: Vec<UtxoId>) {
            self.manager.freeze_utxos(utxos);
        }

        pub fn unfreeze_utxos(&self, utxos: Vec<UtxoId>) {
            self.manager.unfreeze_utxos(utxos);
        }

        pub fn burn_utxos(&self, utxos: Vec<(UtxoId, CommitmentValueProof)>) {
            for (utxo, proof) in utxos {
                self.manager.burn_utxo(utxo, Some(proof));
            }
        }
    }
}
