//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use log::*;
use tari_ootle_transaction::Transaction;

use crate::{TransactionValidationError, Validator};

const LOG_TARGET: &str = "tari::ootle::mempool::validators::inputs_are_declarable";

/// Rejects a transaction that declares as an input a substate no transaction can use as one:
///
/// - one the engine reserves and never stores — a public-key identity, a caller badge, or one of the two resources
///   those badges are namespaced by. Such an input can never resolve: consensus would abort the transaction at input
///   resolution having already sequenced it.
/// - one that only exists as the output of the transaction that created it — a transaction receipt or a burn claim's
///   tombstone. No instruction reads or writes one, so declaring it only locks it and pledges its bytes across shard
///   groups.
///
/// The address alone decides this, with no reference to any node's view of state, so refusing it at ingress cannot
/// false-reject and the cost falls on the sender rather than on a committee.
///
/// Authority over the badges themselves does not rest here — an input reaches `CallScope`, never the
/// `AuthorizationScope` that badges are checked against, and both badge resources are empty by invariant. This
/// rejects the request as nonsense, earlier and with a reason that names the mistake.
#[derive(Debug, Clone, Default)]
pub struct InputsAreDeclarableValidator;

impl InputsAreDeclarableValidator {
    pub fn new() -> Self {
        Self
    }
}

impl Validator<Transaction> for InputsAreDeclarableValidator {
    type Context = ();
    type Error = TransactionValidationError;

    fn validate(&self, _context: &(), transaction: &Transaction) -> Result<(), Self::Error> {
        if let Some(input) = transaction
            .all_inputs_iter()
            .find(|input| !input.substate_id().is_declarable_input())
        {
            let substate_id = input.substate_id().clone();
            let transaction_id = transaction.calculate_id();
            if substate_id.is_virtual() {
                warn!(target: LOG_TARGET, "InputsAreDeclarableValidator - FAIL: reserved substate {substate_id} declared as an input");
                return Err(TransactionValidationError::ReservedSubstateInput {
                    transaction_id,
                    substate_id,
                });
            }
            warn!(target: LOG_TARGET, "InputsAreDeclarableValidator - FAIL: output-only substate {substate_id} declared as an input");
            return Err(TransactionValidationError::OutputOnlySubstateInput {
                transaction_id,
                substate_id,
            });
        }

        debug!(target: LOG_TARGET, "InputsAreDeclarableValidator - OK");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tari_common_types::types::PrivateKey;
    use tari_engine_types::substate::SubstateId;
    use tari_ootle_common_types::Epoch;
    use tari_template_lib::types::{
        Amount,
        ClaimedOutputTombstoneAddress,
        ComponentAddress,
        NonFungibleAddress,
        ObjectKey,
        TransactionReceiptAddress,
        constants::{
            CALLER_COMPONENT_RESOURCE_ADDRESS,
            DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS,
            PUBLIC_IDENTITY_RESOURCE_ADDRESS,
        },
        crypto::RistrettoPublicKeyBytes,
    };

    use super::*;

    fn transaction_with_input(input: SubstateId) -> Transaction {
        Transaction::builder_localnet(Epoch(1))
            .pay_fee_from_component(ComponentAddress::from_array([1u8; 32]), Amount::new(1000))
            .add_input(input)
            .build_and_seal(&PrivateKey::from(1u64))
    }

    fn validate(input: SubstateId) -> Result<(), TransactionValidationError> {
        InputsAreDeclarableValidator::new().validate(&(), &transaction_with_input(input))
    }

    #[test]
    fn it_refuses_a_caller_component_badge() {
        let badge = NonFungibleAddress::caller_component_badge(ComponentAddress::new(ObjectKey::default()));
        assert!(matches!(
            validate(badge.into()),
            Err(TransactionValidationError::ReservedSubstateInput { .. })
        ));
    }

    #[test]
    fn it_refuses_a_direct_caller_template_badge() {
        let badge = NonFungibleAddress::direct_caller_template_badge(Default::default());
        assert!(matches!(
            validate(badge.into()),
            Err(TransactionValidationError::ReservedSubstateInput { .. })
        ));
    }

    /// The badge resources hold nothing and have no substate of their own, so naming the resource is as
    /// unresolvable as naming a badge.
    #[test]
    fn it_refuses_a_badge_resource() {
        assert!(matches!(
            validate(CALLER_COMPONENT_RESOURCE_ADDRESS.into()),
            Err(TransactionValidationError::ReservedSubstateInput { .. })
        ));
        assert!(matches!(
            validate(DIRECT_CALLER_TEMPLATE_RESOURCE_ADDRESS.into()),
            Err(TransactionValidationError::ReservedSubstateInput { .. })
        ));
    }

    /// A signer badge is issued from the transaction's signatures, not resolved from state, so naming one as an
    /// input is the same mistake.
    #[test]
    fn it_refuses_a_public_key_identity() {
        let identity = NonFungibleAddress::from_public_key(RistrettoPublicKeyBytes::default());
        assert!(matches!(
            validate(identity.into()),
            Err(TransactionValidationError::ReservedSubstateInput { .. })
        ));
    }

    /// The identity *resource* is stored in the genesis state even though its non-fungibles are issued from
    /// signatures, so it must stay nameable — every account transaction reaches it through
    /// `get_dependent_substates`, which adds the resource of any non-fungible child.
    #[test]
    fn it_accepts_the_identity_resource() {
        validate(PUBLIC_IDENTITY_RESOURCE_ADDRESS.into()).unwrap();
    }

    #[test]
    fn it_refuses_a_transaction_receipt() {
        let receipt = TransactionReceiptAddress::from_array([3u8; 32]);
        assert!(matches!(
            validate(receipt.into()),
            Err(TransactionValidationError::OutputOnlySubstateInput { .. })
        ));
    }

    #[test]
    fn it_refuses_a_claimed_output_tombstone() {
        let tombstone = ClaimedOutputTombstoneAddress::new(ObjectKey::from_array([4u8; 32]));
        assert!(matches!(
            validate(tombstone.into()),
            Err(TransactionValidationError::OutputOnlySubstateInput { .. })
        ));
    }

    #[test]
    fn it_accepts_a_real_substate() {
        validate(ComponentAddress::from_array([2u8; 32]).into()).unwrap();
    }
}
