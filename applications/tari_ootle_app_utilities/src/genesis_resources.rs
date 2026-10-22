//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::resource::Resource;
use tari_ootle_transaction::Network;
use tari_template_lib::{
    prelude::LOCKED,
    types::{
        Metadata,
        ResourceAddress,
        ResourceType,
        SubstateOwnerRule,
        access_rules::ResourceAccessRules,
        constants::{
            NFT_FAUCET_COMPONENT_ADDRESS,
            NFT_FAUCET_RESOURCE_ADDRESS,
            PUBLIC_IDENTITY_RESOURCE_ADDRESS,
            STEALTH_TARI_RESOURCE_ADDRESS,
            TOKEN_SYMBOL,
            XTR_FAUCET_CLAIM_RESOURCE_ADDRESS,
            XTR_FAUCET_COMPONENT_ADDRESS,
        },
        rule,
    },
};

pub fn get_public_identity_resource() -> (ResourceAddress, Resource) {
    let value = Resource::new(
        ResourceType::NonFungible,
        SubstateOwnerRule::None,
        ResourceAccessRules::new(),
        Metadata::from([(TOKEN_SYMBOL, "ID".to_string())]),
        None,
        None,
        0,
        false,
    );
    (PUBLIC_IDENTITY_RESOURCE_ADDRESS, value)
}

/// The tTARI faucet's claim receipt resource: one NFT per claimant public key, burned as soon as it is minted. The
/// burned substate key persists on-chain, preventing duplicate claims.
pub fn get_xtr_faucet_claim_resource() -> (ResourceAddress, Resource) {
    let resource = Resource::new(
        ResourceType::NonFungible,
        SubstateOwnerRule::None,
        ResourceAccessRules::new()
            .mintable(rule!(component(XTR_FAUCET_COMPONENT_ADDRESS)), LOCKED)
            .burnable(rule!(component(XTR_FAUCET_COMPONENT_ADDRESS)), LOCKED),
        Metadata::new(),
        None,
        None,
        0,
        false,
    );
    (XTR_FAUCET_CLAIM_RESOURCE_ADDRESS, resource)
}

pub fn get_nft_faucet_resource() -> (ResourceAddress, Resource) {
    let resource = Resource::new(
        ResourceType::NonFungible,
        SubstateOwnerRule::None,
        ResourceAccessRules::new().mintable(rule!(component(NFT_FAUCET_COMPONENT_ADDRESS)), LOCKED),
        Metadata::from([("name", "NFT Faucet"), (TOKEN_SYMBOL, "tNFT")]),
        None,
        None,
        0,
        true,
    );
    (NFT_FAUCET_RESOURCE_ADDRESS, resource)
}

pub fn get_stealth_tari_resource(network: Network) -> (ResourceAddress, Resource) {
    let symbol = if network.is_testnet() { "tTARI" } else { "TARI" };
    let xtr_resource = Resource::new(
        ResourceType::Stealth,
        SubstateOwnerRule::None,
        ResourceAccessRules::new()
            // These are defaults, but just for explicitness
            .mintable(rule!(deny_all), LOCKED)
            .burnable(rule!(deny_all), LOCKED)
            .recallable(rule!(deny_all), LOCKED)
            .freezable(rule!(deny_all), LOCKED),
        Metadata::from([(TOKEN_SYMBOL, symbol)]),
        None,
        None,
        6,
        // Disable total supply tracking for XTR. This is because it is not feasible to include "the fee exhaust" in
        // the tracking (as that would require mutating the resource on every transaction). Tracking supply can
        // be done by summing up the total burn claims (ClaimedOutputTombstone) and subtracting the total exhaust in
        // fee receipts.
        false,
    );
    (STEALTH_TARI_RESOURCE_ADDRESS, xtr_resource)
}

#[cfg(test)]
mod tests {
    use tari_engine_types::{Epoch, SubstateVersion, substate::hash_substate};
    use tari_template_lib::types::{AccessRule, access_rules::ResourceAuthAction};

    use super::*;

    /// Pins Esmeralda's genesis resources to the substate hashes its genesis state commits to. A change to any of
    /// them changes Esmeralda's genesis state roots.
    #[test]
    fn esmeralda_genesis_resource_hashes_are_pinned() {
        let hashes = [
            get_public_identity_resource(),
            get_stealth_tari_resource(Network::Esmeralda),
            get_xtr_faucet_claim_resource(),
            get_nft_faucet_resource(),
        ]
        .map(|(_, resource)| {
            hash_substate(
                Network::Esmeralda,
                &resource.into(),
                SubstateVersion::ZERO,
                Epoch::zero(),
            )
            .to_string()
        });
        assert_eq!(hashes, [
            "047cd7d24d968b1406a56bc059f167325c9d4c523cc88cec8663b677dcccc265",
            "e26c4af38b0ef0f897aa73f93efd47c823d7524b117ec2977b88b182e71459e1",
            "fc9ce6405c8b3389bdce7f6d267ed93ceabcf464a9dde35ac005c70f3dfdc499",
            "1ef457c8d757984f5fbab3773160ce134ec24cc80bba35b3876270daf0025498",
        ]);
    }

    #[test]
    fn genesis_resources_deny_nft_data_updates() {
        for (address, resource) in [
            get_public_identity_resource(),
            get_xtr_faucet_claim_resource(),
            get_nft_faucet_resource(),
        ] {
            assert_eq!(
                *resource
                    .access_rules()
                    .get_access_rule(&ResourceAuthAction::UpdateNonFungibleData),
                AccessRule::DenyAll,
                "{address}"
            );
        }
    }
}
