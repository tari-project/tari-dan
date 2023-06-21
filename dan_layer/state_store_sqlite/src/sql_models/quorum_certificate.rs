//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use diesel::Queryable;
use tari_dan_storage::{consensus_models, StorageError};
use time::PrimitiveDateTime;

use crate::serialization::deserialize_json;

#[derive(Debug, Clone, Queryable)]
pub struct QuorumCertificate {
    pub id: i32,
    pub qc_id: String,
    pub json: String,
    pub created_at: PrimitiveDateTime,
}

impl TryFrom<QuorumCertificate> for consensus_models::QuorumCertificate {
    type Error = StorageError;

    fn try_from(value: QuorumCertificate) -> Result<Self, Self::Error> {
        deserialize_json(&value.json)
    }
}
