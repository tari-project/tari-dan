//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::published_template::PublishedTemplateMetadata;
use tari_indexer_client::types::TemplateCatalogueItem;
use tari_ootle_storage::time::PrimitiveDateTime;
use tari_ootle_template_metadata::MetadataHash;
use tari_template_lib_types::{Hash32, TemplateAddress, crypto::RistrettoPublicKeyBytes};

use crate::storage_sqlite::schema::template_catalogue;

/// A row in the `template_catalogue` SQLite table.
#[derive(Debug, Clone, Identifiable, Queryable)]
#[diesel(table_name = template_catalogue)]
pub(crate) struct TemplateCatalogueRow {
    pub id: i64,
    pub template_address: String,
    pub template_name: String,
    pub author_public_key: String,
    pub binary_hash: String,
    pub at_epoch: i64,
    pub metadata_hash: Option<String>,
    #[allow(dead_code)]
    pub created_at: PrimitiveDateTime,
    #[allow(dead_code)]
    pub updated_at: PrimitiveDateTime,
}

/// Insertable model used for upserts.
#[derive(Debug, Insertable, AsChangeset)]
#[diesel(table_name = template_catalogue)]
pub(crate) struct NewTemplateCatalogueRow {
    pub template_address: String,
    pub template_name: String,
    pub author_public_key: String,
    pub binary_hash: String,
    pub at_epoch: i64,
    pub metadata_hash: Option<String>,
}

/// Public type exposed through the store trait and REST API.
#[derive(Debug, Clone)]
pub struct TemplateCatalogueEntry {
    pub id: i64,
    pub template_address: TemplateAddress,
    pub template_name: String,
    pub author_public_key: RistrettoPublicKeyBytes,
    pub binary_hash: Hash32,
    pub at_epoch: u64,
    pub metadata_hash: Option<MetadataHash>,
}

impl TryFrom<TemplateCatalogueRow> for TemplateCatalogueEntry {
    type Error = anyhow::Error;

    fn try_from(row: TemplateCatalogueRow) -> Result<Self, Self::Error> {
        Ok(Self {
            id: row.id,
            template_address: TemplateAddress::from_hex(&row.template_address)
                .map_err(|e| anyhow::anyhow!("Invalid template_address hex: {e}"))?,
            template_name: row.template_name,
            author_public_key: RistrettoPublicKeyBytes::from_bytes(
                &hex::decode(&row.author_public_key)
                    .map_err(|e| anyhow::anyhow!("Invalid author_public_key hex: {e}"))?,
            )
            .map_err(|_| anyhow::anyhow!("Invalid author_public_key bytes"))?,
            binary_hash: Hash32::from_hex(&row.binary_hash)
                .map_err(|e| anyhow::anyhow!("Invalid binary_hash hex: {e}"))?,
            at_epoch: row.at_epoch as u64,
            metadata_hash: row
                .metadata_hash
                .map(|h| MetadataHash::from_hex(&h))
                .transpose()
                .map_err(|e| anyhow::anyhow!("Invalid metadata_hash hex: {e}"))?,
        })
    }
}

impl From<TemplateCatalogueEntry> for TemplateCatalogueItem {
    fn from(e: TemplateCatalogueEntry) -> Self {
        Self {
            template_address: e.template_address,
            template_name: e.template_name,
            author_public_key: e.author_public_key,
            binary_hash: e.binary_hash,
            at_epoch: e.at_epoch,
            metadata_hash: e.metadata_hash,
        }
    }
}

impl From<(TemplateAddress, &PublishedTemplateMetadata)> for NewTemplateCatalogueRow {
    fn from((address, meta): (TemplateAddress, &PublishedTemplateMetadata)) -> Self {
        Self {
            template_address: hex::encode(address.as_slice()),
            template_name: meta.template_name.clone(),
            author_public_key: hex::encode(meta.author_public_key.as_bytes()),
            binary_hash: hex::encode(meta.binary_hash.as_slice()),
            at_epoch: meta.at_epoch as i64,
            metadata_hash: meta.metadata_hash.as_ref().map(|h| h.to_hex()),
        }
    }
}
