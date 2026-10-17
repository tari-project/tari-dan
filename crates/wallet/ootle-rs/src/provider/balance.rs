//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashMap;

use ootle_byte_type::ConvertFromByteType;
use tari_crypto::ristretto::RistrettoSecretKey;
use tari_indexer_client::types::MAX_SUBSTATES_PER_FETCH;
use tari_ootle_common_types::engine_types::{
    Utxo,
    crypto::{ElgamalVerifiableBalance, ValueLookup},
    indexed_value::IndexedWellKnownTypes,
    substate::{Substate, SubstateId},
};
use tari_template_lib_types::{Amount, ComponentAddress, ResourceAddress, UtxoAddress, UtxoId, VaultId};

use crate::provider::{ProviderError, ProviderResult, indexer::IndexerProvider};

/// Balance information for a single vault
#[derive(Debug, Clone)]
pub struct VaultBalance {
    pub vault_id: VaultId,
    pub resource_address: ResourceAddress,
    pub balance: Amount,
    pub locked_balance: Amount,
}

/// Per-UTXO outcome of decrypting a stealth output's viewable balance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StealthUtxoValue {
    /// Successfully decrypted plaintext value (microtari).
    Value(u64),
    /// The viewable-balance ciphertext is valid but its value is outside the lookup's coverage.
    /// Retryable with a wider lookup file (assuming the view key is correct for the resource).
    OutOfRange,
    /// The output carries no viewable-balance proof, so it cannot be decrypted.
    NoViewableBalance,
    /// The viewable-balance proof bytes could not be decompressed to a valid ElGamal ciphertext.
    MalformedProof,
}

impl<Wallet> IndexerProvider<Wallet> {
    /// Returns the balance for a specific resource in the given account.
    ///
    /// Fetches the account component substate, extracts all vault IDs from its state,
    /// then fetches each vault to find the one matching the requested resource address.
    /// Returns `Amount::zero()` if no vault exists for the given resource.
    pub async fn get_account_balance(
        &self,
        account: ComponentAddress,
        resource: ResourceAddress,
    ) -> ProviderResult<Amount> {
        let vaults = self.fetch_account_vaults(account).await?;
        let balance = vaults
            .into_iter()
            .find(|v| v.resource_address == resource)
            .map(|v| v.balance)
            .unwrap_or_else(Amount::zero);
        Ok(balance)
    }

    /// Returns balances for all resources held in the given account.
    ///
    /// Fetches the account component and all its vaults, returning a map
    /// from resource address to balance amount.
    pub async fn get_account_balances(
        &self,
        account: ComponentAddress,
    ) -> ProviderResult<HashMap<ResourceAddress, Amount>> {
        let vaults = self.fetch_account_vaults(account).await?;
        let balances = vaults.into_iter().map(|v| (v.resource_address, v.balance)).collect();
        Ok(balances)
    }

    /// Decrypts the value of one or more stealth UTXOs using the given ElGamal view secret key and a value
    /// lookup.
    ///
    /// Each UTXO must contain a viewable balance proof. Returns a map from UTXO address to the decrypted value;
    /// UTXOs without a viewable balance proof, or whose value is not covered by the lookup, are omitted.
    pub fn decrypt_stealth_utxo_values<L: ValueLookup>(
        &self,
        view_secret_key: &RistrettoSecretKey,
        utxo_substates: &HashMap<SubstateId, Substate>,
        lookup: &L,
    ) -> ProviderResult<HashMap<UtxoAddress, u64>> {
        let proofs = utxo_substates
            .iter()
            .filter_map(|(id, s)| {
                let addr = id.as_utxo_address()?;
                let proof = s
                    .substate_value()
                    .as_utxo()
                    .and_then(|u| u.output())
                    .and_then(|o| o.output.viewable_balance.as_ref())?;
                Some((addr, proof))
            })
            .collect::<Vec<_>>();

        if proofs.is_empty() {
            return Ok(HashMap::new());
        }

        let addresses = proofs.iter().map(|(a, _)| a.clone()).collect::<Vec<_>>();
        let elgamal_proofs = proofs
            .iter()
            .map(|(_, p)| ElgamalVerifiableBalance::convert_from_byte_type(p))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| ProviderError::other(format!("Failed to decompress viewable balance proof: {e}")))?;

        let results = ElgamalVerifiableBalance::decrypt_many(view_secret_key, &elgamal_proofs, lookup)
            .map_err(|e| ProviderError::other(format!("Value lookup error: {e}")))?;

        let values = addresses
            .into_iter()
            .zip(results)
            .filter_map(|(addr, val)| val.map(|v| (addr, v)))
            .collect();

        Ok(values)
    }

    /// Decrypts the viewable balance of each stealth UTXO, preserving a per-UTXO [`StealthUtxoValue`]
    /// in input order.
    ///
    /// Unlike [`Self::decrypt_stealth_utxo_values`], this does not silently drop UTXOs whose value is
    /// outside the lookup's coverage or that lack a proof — a monitor can therefore record
    /// `OutOfRange` (retryable with a wider lookup) distinctly from a decrypted value. The expensive
    /// batch lookup runs only over well-formed ciphertexts.
    ///
    /// The view key is not validated against any resource here; a key that does not match the
    /// resource's view key yields `OutOfRange` for every output, so validate it against
    /// `resource.view_key()` beforehand to disambiguate `OutOfRange` from a key mismatch.
    pub fn decrypt_stealth_utxo_values_opt<L: ValueLookup>(
        &self,
        view_secret_key: &RistrettoSecretKey,
        utxos: &[(UtxoId, Utxo)],
        lookup: &L,
    ) -> ProviderResult<Vec<(UtxoId, StealthUtxoValue)>> {
        let mut outcomes: Vec<Option<StealthUtxoValue>> = Vec::with_capacity(utxos.len());
        let mut proofs = Vec::new();
        let mut proof_indexes = Vec::new();

        for (i, (_, utxo)) in utxos.iter().enumerate() {
            match utxo.output().and_then(|o| o.output.viewable_balance.as_ref()) {
                None => outcomes.push(Some(StealthUtxoValue::NoViewableBalance)),
                Some(bytes) => match ElgamalVerifiableBalance::convert_from_byte_type(bytes) {
                    Ok(proof) => {
                        outcomes.push(None);
                        proof_indexes.push(i);
                        proofs.push(proof);
                    },
                    Err(_) => outcomes.push(Some(StealthUtxoValue::MalformedProof)),
                },
            }
        }

        if !proofs.is_empty() {
            let results = ElgamalVerifiableBalance::decrypt_many(view_secret_key, &proofs, lookup)
                .map_err(|e| ProviderError::other(format!("Value lookup error: {e}")))?;
            for (index, value) in proof_indexes.into_iter().zip(results) {
                outcomes[index] = Some(match value {
                    Some(v) => StealthUtxoValue::Value(v),
                    None => StealthUtxoValue::OutOfRange,
                });
            }
        }

        Ok(utxos
            .iter()
            .zip(outcomes)
            .map(|((id, _), outcome)| (*id, outcome.expect("outcome assigned for every utxo index")))
            .collect())
    }

    /// Fetches a UTXO from the network and decrypts its value using the given ElGamal view secret key.
    ///
    /// This is a convenience method that fetches a single UTXO substate and decrypts its viewable balance.
    /// Returns `None` if the UTXO does not contain a viewable balance proof or its value is not covered by the
    /// lookup.
    pub async fn get_utxo_value<L: ValueLookup>(
        &self,
        view_secret_key: &RistrettoSecretKey,
        utxo_address: UtxoAddress,
        lookup: &L,
    ) -> ProviderResult<Option<u64>> {
        let substate = self.fetch_substate(SubstateId::from(utxo_address)).await?;

        let proof = substate
            .substate_value()
            .as_utxo()
            .and_then(|u| u.output())
            .and_then(|o| o.output.viewable_balance.as_ref());

        let Some(proof) = proof else {
            return Ok(None);
        };

        let balance = ElgamalVerifiableBalance::convert_from_byte_type(proof)
            .map_err(|e| ProviderError::other(format!("Failed to decompress viewable balance proof: {e}")))?;

        balance
            .decrypt(view_secret_key, lookup)
            .map_err(|e| ProviderError::other(format!("Value lookup error: {e}")))
    }

    /// Fetches all vault balances for the given account component.
    async fn fetch_account_vaults(&self, account: ComponentAddress) -> ProviderResult<Vec<VaultBalance>> {
        let substate = self.fetch_substate(account).await?;
        let component = substate
            .substate_value()
            .component()
            .ok_or_else(|| ProviderError::other("Expected component substate for account"))?;

        let indexed = IndexedWellKnownTypes::from_value(component.state())
            .map_err(|e| ProviderError::other(format!("Failed to index component state: {e}")))?;

        let vault_ids = indexed.vault_ids();
        if vault_ids.is_empty() {
            return Ok(Vec::new());
        }

        let vault_substate_ids: Vec<SubstateId> = vault_ids.iter().copied().map(SubstateId::Vault).collect();
        let mut substates = HashMap::with_capacity(vault_substate_ids.len());
        for batch in vault_substate_ids.chunks(MAX_SUBSTATES_PER_FETCH) {
            substates.extend(self.verifier().fetch_substates(batch).await?);
        }

        let mut balances = Vec::with_capacity(substates.len());
        for (id, substate) in substates {
            let substate: Substate = substate;
            let vault = substate.into_substate_value().into_vault().ok_or_else(|| {
                ProviderError::other(format!(
                    "Expected vault substate for {id}, but got a different substate type"
                ))
            })?;
            balances.push(VaultBalance {
                vault_id: id.as_vault_id().expect("SubstateId::Vault always has a vault id"),
                resource_address: *vault.resource_address(),
                balance: vault.balance(),
                locked_balance: vault.locked_balance(),
            });
        }

        Ok(balances)
    }
}
