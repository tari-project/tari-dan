//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::substate::SubstateId;
use tari_template_lib_types::TemplateAddress;

use crate::storage_sqlite::schema::watched_substates;

#[derive(Debug, Clone, Queryable)]
#[diesel(table_name = watched_substates)]
pub(crate) struct WatchedSubstateRow {
    #[allow(dead_code)]
    pub id: i64,
    pub component_address: String,
    pub template_address: String,
    #[allow(dead_code)]
    pub created_at: tari_ootle_storage::time::PrimitiveDateTime,
}

#[derive(Debug, Insertable)]
#[diesel(table_name = watched_substates)]
pub(crate) struct NewWatchedSubstate<'a> {
    pub component_address: &'a str,
    pub template_address: &'a str,
}

#[derive(Debug, Clone)]
pub struct WatchedSubstateEntry {
    pub component_address: SubstateId,
    pub template_address: TemplateAddress,
}
