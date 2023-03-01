//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::convert::TryFrom;

use tari_common_types::types::{PrivateKey, PublicKey};
use tari_crypto::{keys::PublicKey as PublicKeyTrait, ristretto::RistrettoPublicKey};
use tari_dan_common_types::ShardId;
use tari_engine_types::{instruction::Instruction, substate::SubstateAddress};
use tari_template_lib::models::{
    AddressListId,
    AddressListItemAddress,
    NonFungibleAddress,
    NonFungibleId,
    ResourceAddress,
};

use super::Transaction;
use crate::{
    change::SubstateChange,
    id_provider::IdProvider,
    transaction::TransactionMeta,
    InstructionSignature,
    ObjectClaim,
};

#[derive(Debug, Clone, Default)]
pub struct TransactionBuilder {
    instructions: Vec<Instruction>,
    fee: u64,
    meta: TransactionMeta,
    signature: Option<InstructionSignature>,
    sender_public_key: Option<RistrettoPublicKey>,
    new_non_fungible_outputs: Vec<(ResourceAddress, u8)>,
    new_address_list_item_outputs: Vec<(AddressListId, u64)>,
}

impl TransactionBuilder {
    pub fn new() -> Self {
        Self {
            instructions: Vec::new(),
            signature: None,
            sender_public_key: None,
            fee: 0,
            meta: TransactionMeta::default(),
            new_non_fungible_outputs: vec![],
            new_address_list_item_outputs: vec![],
        }
    }

    pub fn with_fee(&mut self, fee: u64) -> &mut Self {
        self.fee = fee;
        self
    }

    pub fn add_instruction(&mut self, instruction: Instruction) -> &mut Self {
        self.instructions.push(instruction);
        // Reset the signature as it is no longer valid
        self.signature = None;
        self
    }

    pub fn with_instructions(&mut self, instructions: Vec<Instruction>) -> &mut Self {
        self.instructions.extend(instructions);
        // Reset the signature as it is no longer valid
        self.signature = None;
        self
    }

    pub fn with_signature(&mut self, signature: InstructionSignature) -> &mut Self {
        self.signature = Some(signature);
        self
    }

    pub fn with_sender_public_key(&mut self, sender_public_key: RistrettoPublicKey) -> &mut Self {
        self.sender_public_key = Some(sender_public_key);
        self
    }

    pub fn sign(&mut self, secret_key: &PrivateKey) -> &mut Self {
        self.signature = Some(InstructionSignature::sign(secret_key, &self.instructions));
        self.sender_public_key = Some(PublicKey::from_secret_key(secret_key));
        self
    }

    /// Add an input to be consumed
    pub fn add_input(&mut self, input_object: ShardId) -> &mut Self {
        self.meta
            .involved_objects_mut()
            .insert(input_object, (SubstateChange::Destroy, ObjectClaim {}));
        self
    }

    pub fn with_inputs(&mut self, inputs: Vec<ShardId>) -> &mut Self {
        for input in inputs {
            self.add_input(input);
        }
        self
    }

    pub fn with_outputs(&mut self, outputs: Vec<ShardId>) -> &mut Self {
        for output in outputs {
            self.add_output(output);
        }
        self
    }

    pub fn add_output(&mut self, output_object: ShardId) -> &mut Self {
        self.meta
            .involved_objects_mut()
            .insert(output_object, (SubstateChange::Create, ObjectClaim {}));
        self
    }

    pub fn with_new_outputs(&mut self, num_outputs: u8) -> &mut Self {
        self.meta.set_max_outputs(num_outputs.into());
        self
    }

    pub fn with_new_non_fungible_outputs(&mut self, new_non_fungible_outputs: Vec<(ResourceAddress, u8)>) -> &mut Self {
        self.new_non_fungible_outputs = new_non_fungible_outputs;
        self
    }

    pub fn with_new_address_list_item_outputs(
        &mut self,
        new_address_list_item_outputs: Vec<(AddressListId, u64)>,
    ) -> &mut Self {
        self.new_address_list_item_outputs = new_address_list_item_outputs;
        self
    }

    pub fn build(mut self) -> Transaction {
        let mut transaction = Transaction::new(
            self.fee,
            self.instructions.drain(..).collect(),
            self.signature.take().expect("not signed"),
            self.sender_public_key.take().expect("not signed"),
            self.meta,
        );

        let max_outputs = transaction.meta().max_outputs();
        let total_new_nft_outputs = self
            .new_non_fungible_outputs
            .iter()
            .map(|(_, count)| u32::from(*count))
            .sum::<u32>();
        let id_provider = IdProvider::new(*transaction.hash(), max_outputs + total_new_nft_outputs);

        transaction
            .meta_mut()
            .involved_objects_mut()
            .extend((0..max_outputs).map(|_| {
                let new_hash = id_provider
                    .new_address_hash()
                    .expect("id provider provides num_outputs IDs");
                (
                    ShardId::from_hash(&new_hash, 0),
                    (SubstateChange::Create, ObjectClaim {}),
                )
            }));

        let mut new_nft_outputs =
            Vec::with_capacity(usize::try_from(total_new_nft_outputs).expect("too many new NFT outputs"));
        for (resource_addr, count) in self.new_non_fungible_outputs {
            new_nft_outputs.extend((0..count).map({
                |_| {
                    let new_hash = id_provider.new_uuid().expect("id provider provides num_outputs IDs");
                    let address = NonFungibleAddress::new(resource_addr, NonFungibleId::from_u256(new_hash));
                    let new_addr = SubstateAddress::NonFungible(address);
                    (
                        ShardId::from_hash(&new_addr.to_canonical_hash(), 0),
                        (SubstateChange::Create, ObjectClaim {}),
                    )
                }
            }));
        }

        transaction.meta_mut().involved_objects_mut().extend(new_nft_outputs);

        // add the involved objects for address list items
        let new_item_outputs: Vec<(ShardId, (SubstateChange, ObjectClaim))> = self
            .new_address_list_item_outputs
            .iter()
            .map(|(list_id, index)| {
                let item_addr = AddressListItemAddress::new(*list_id, *index);
                let substate_addr = SubstateAddress::AddressListItem(item_addr);
                let shard_id = ShardId::from_hash(&substate_addr.to_canonical_hash(), 0);

                (shard_id, (SubstateChange::Create, ObjectClaim {}))
            })
            .collect();
        transaction.meta_mut().involved_objects_mut().extend(new_item_outputs);

        transaction
    }
}
