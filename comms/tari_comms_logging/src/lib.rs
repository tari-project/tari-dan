//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

#[macro_use]
extern crate diesel;
#[macro_use]
extern crate diesel_migrations;

mod sqlite_message_log;
pub use sqlite_message_log::SqliteMessageLog;
mod schema;
