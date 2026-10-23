//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use axum_extra::headers::authorization::Bearer;
use blake2::{Blake2b512, Digest};
use ootle_byte_type::ToByteType;
use tari_crypto::{
    keys::PublicKey as _,
    ristretto::{RistrettoPublicKey, RistrettoSchnorr},
};
use tari_ootle_common_types::optional::Optional;
use tari_ootle_wallet_sdk::network::WalletNetworkInterface;
use tari_ootle_walletd_client::{
    permissions::{Crud, Permission},
    types::{
        AuthoredTemplate,
        SignTemplateMetadataRequest,
        SignTemplateMetadataResponse,
        TemplatesGetRequest,
        TemplatesGetResponse,
        TemplatesListAuthoredRequest,
        TemplatesListAuthoredResponse,
    },
};
use tari_template_lib_types::crypto::Scalar32Bytes;
use tari_utilities::ByteArray;

use crate::handlers::{
    HandlerContext,
    helpers::{ensure_signing_key, not_found},
};

pub async fn handle_get(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TemplatesGetRequest,
) -> Result<TemplatesGetResponse, anyhow::Error> {
    let sdk = context.wallet_sdk().clone();
    context.authorize(token, &[Permission::Templates(Crud::Read)])?;

    if let Some(template) = sdk
        .template_api()
        .fetch_authored_template(req.template_address)
        .optional()?
    {
        return Ok(TemplatesGetResponse {
            template_definition: template.into(),
        });
    }

    let template_definition = sdk
        .get_network_interface()
        .fetch_template_definition(req.template_address)
        .await
        .optional()?
        .ok_or_else(|| not_found(format!("Template not found at address {}", req.template_address)))?;

    Ok(TemplatesGetResponse { template_definition })
}

pub async fn handle_list_owned(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: TemplatesListAuthoredRequest,
) -> Result<TemplatesListAuthoredResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Templates(Crud::Read)])?;

    let (templates, total_templates) = context.wallet_sdk().template_api().list_templates(
        req.author_public_key.as_ref(),
        req.page.into(),
        req.page_size.into(),
    )?;
    Ok(TemplatesListAuthoredResponse {
        templates: templates.into_iter().map(AuthoredTemplate::from).collect(),
        total_templates,
    })
}

const SIGN_METADATA_DOMAIN: &[u8] = b"com.tari.ootle.community.SignedMetadataUpdate";

pub async fn handle_sign_metadata(
    context: &HandlerContext,
    token: Option<&Bearer>,
    req: SignTemplateMetadataRequest,
) -> Result<SignTemplateMetadataResponse, anyhow::Error> {
    context.authorize(token, &[Permission::Templates(Crud::Create)])?;

    let sdk = context.wallet_sdk();
    ensure_signing_key(sdk, "key_id", &req.key_id)?;

    let key = sdk.key_manager_api().get_key(req.key_id)?;
    let author_public_key = RistrettoPublicKey::from_secret_key(&key.secret);

    // CBOR-encode the metadata
    let metadata_cbor = req.metadata.to_cbor()?;
    let metadata_hash = req.metadata.hash()?;

    // Generate random nonce keypair
    let (nonce_secret, nonce_public) = RistrettoPublicKey::random_keypair(&mut rand::rng());

    // Compute Blake2b-512 challenge
    let challenge = Blake2b512::new()
        .chain_update(SIGN_METADATA_DOMAIN)
        .chain_update(nonce_public.as_bytes())
        .chain_update(author_public_key.as_bytes())
        .chain_update(req.template_address.as_ref())
        .chain_update(&metadata_cbor)
        .finalize();

    // Sign
    let sig = RistrettoSchnorr::sign_raw_uniform(&key.secret, nonce_secret, &challenge)
        .map_err(|e| anyhow::anyhow!("Schnorr signing failed: {e}"))?;

    Ok(SignTemplateMetadataResponse {
        public_nonce: sig.get_public_nonce().to_byte_type(),
        signature: Scalar32Bytes::from_bytes(sig.get_signature().as_bytes())?,
        public_key: author_public_key.to_byte_type(),
        metadata_cbor,
        metadata_hash,
    })
}

#[cfg(test)]
mod tests {
    use tari_ootle_template_metadata::TemplateMetadata;
    use tari_ootle_wallet_sdk::models::{KeyBranch, KeyId};
    use tari_template_lib_types::TemplateAddress;

    use super::*;
    use crate::handlers::test_support::TestDaemon;

    fn sign_request(key_id: KeyId) -> SignTemplateMetadataRequest {
        SignTemplateMetadataRequest {
            key_id,
            template_address: TemplateAddress::from_array([0x11; 32]),
            metadata: TemplateMetadata::new("template".to_string(), "1.0.0".to_string()),
        }
    }

    #[tokio::test]
    async fn metadata_is_signed_only_with_a_signing_key() {
        let daemon = TestDaemon::start().await;
        let author = daemon.api_key("author", "templates:create");

        for branch in [
            KeyBranch::ConfidentialMask,
            KeyBranch::StealthMask,
            KeyBranch::ViewOnlyKey,
            KeyBranch::ElgamalEncryptionViewKey,
        ] {
            let err = handle_sign_metadata(&daemon.context, Some(&author), sign_request(KeyId::derived(branch, 0)))
                .await
                .unwrap_err();
            assert!(err.to_string().contains("key_id"), "{err}");
        }

        handle_sign_metadata(
            &daemon.context,
            Some(&author),
            sign_request(KeyId::derived(KeyBranch::Account, 0)),
        )
        .await
        .unwrap();
    }
}
