//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::collections::HashSet;

use tari_ootle_common_types::{Epoch, InputDeclaration};
use tari_ootle_transaction::{Blob, TransactionBuilder, UnsignedTransaction, args};
use tari_template_lib_types::{Amount, ResourceAddress, TemplateMetadataHash, constants::TARI_TOKEN};

use crate::{
    Address,
    ToAccountAddress,
    builtin_templates::traits::UnsignedTransactionBuilder,
    provider::{Provider, ProviderError, WantInput},
};

/// Alias for [`AccountInvokeBuilder`].
pub type IAccount<'a, P> = AccountInvokeBuilder<'a, P>;

/// Builder for constructing transactions against the built-in Account template.
///
/// Supports public transfers, fee payment, and template publishing.
///
/// ```rust,ignore
/// let tx = IAccount::new(&provider)
///     .pay_fee(1000u64)
///     .public_transfer(&recipient, TARI_TOKEN, 1_000_000u64)
///     .prepare()
///     .await?;
/// ```
pub struct AccountInvokeBuilder<'a, P> {
    builder: TransactionBuilder,
    provider: &'a P,
    want_list: HashSet<WantInput>,
}

impl<'a, P: Provider> UnsignedTransactionBuilder for AccountInvokeBuilder<'a, P> {
    fn default_signer_address(&self) -> &Address {
        self.provider.default_signer_address()
    }

    fn add_input<S: Into<InputDeclaration>>(mut self, substate_id: S) -> Self {
        self.builder = self.builder.add_input(substate_id);
        self
    }

    async fn prepare(self) -> Result<UnsignedTransaction, ProviderError> {
        let Self {
            builder,
            provider,
            want_list,
            ..
        } = self;
        let unsigned_tx = builder.build_unsigned();
        let unsigned_tx = provider.resolve_input_want_list(unsigned_tx, &want_list).await?;
        Ok(unsigned_tx)
    }
}

impl<'a, P: Provider> AccountInvokeBuilder<'a, P> {
    /// `max_epoch` is the last epoch the built transaction may be sequenced in. It is required:
    /// every transaction carries a bounded validity window.
    pub fn new(provider: &'a P, max_epoch: Epoch) -> Self {
        let network = provider.network();
        Self {
            builder: TransactionBuilder::new(network, max_epoch).with_auto_fill_inputs(),
            provider,
            want_list: HashSet::new(),
        }
    }

    pub fn pay_fee<A: Into<Amount>>(mut self, amount: A) -> Self {
        let component_addr = self.default_signer_address().to_account_address();
        self.want_list.insert(WantInput::VaultForResource {
            component_address: component_addr,
            resource_address: TARI_TOKEN,
            required: true,
        });
        self.builder = self.builder.pay_fee_from_component(component_addr, amount);
        self
    }

    fn next_bucket_name(&mut self) -> String {
        format!("__AccountInvokeBuilder_{}", self.builder.next_workspace_id())
    }

    pub fn public_transfer<A: Into<Amount>>(
        mut self,
        to: &Address,
        resource_address: ResourceAddress,
        amount: A,
    ) -> Self {
        let amount: Amount = amount.into();
        if !amount.is_positive() {
            panic!("Transfer amount must be positive");
        }
        let from_component_addr = self.default_signer_address().to_account_address();
        let to_component_addr = to.to_account_address();
        let bucket_name = self.next_bucket_name();
        // We need the vault to pay from
        self.want_list.insert(WantInput::VaultForResource {
            component_address: from_component_addr,
            resource_address,
            required: true,
        });
        // We need the substate of the recipient account to deposit into, if it exists
        self.want_list.insert(WantInput::SpecificSubstate {
            substate_id: to_component_addr.into(),
            required: false,
        });
        // We need the vault to deposit into, if it exists
        self.want_list.insert(WantInput::VaultForResource {
            component_address: to_component_addr,
            resource_address,
            // If it doesn't exist, the  CreateAccount instruction will create it
            required: false,
        });

        self.builder = self
            .builder
            .call_method(from_component_addr, "withdraw", args![resource_address, amount])
            .put_last_instruction_output_on_workspace(&bucket_name)
            .create_account_with_bucket(*to.account_public_key(), bucket_name);
        self
    }

    // TODO: move common builder code into a builder trait
    /// Publish a WASM template by passing the binary directly. The binary is auto-registered
    /// as an unnamed blob.
    pub fn publish_template<T: Into<Blob>>(mut self, template: T) -> Self {
        self.builder = self.builder.publish_template(template);
        self
    }

    /// Publish a WASM template with an off-chain metadata hash.
    pub fn publish_template_with_metadata<T: Into<Blob>, H: Into<TemplateMetadataHash>>(
        mut self,
        template: T,
        metadata_hash: H,
    ) -> Self {
        self.builder = self.builder.publish_template_with_metadata(template, metadata_hash);
        self
    }
}
