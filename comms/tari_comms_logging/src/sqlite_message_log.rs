//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{fs::create_dir_all, path::PathBuf};

use diesel::prelude::*;
use log::error;
use serde::Serialize;

use crate::schema::*;

const LOG_TARGET: &str = "tari::comms::logging::sqlite_message_log";

// Note: this struct does not produce errors because it is for logging. Logs will be output on errors

#[derive(Debug, Insertable)]
#[table_name = "outbound_messages"]
struct NewOutboundMessage {
    destination_type: String,
    destination_pubkey: Vec<u8>,
    message_type: String,
    message_json: String,
}

#[derive(Debug, Insertable)]
#[table_name = "inbound_messages"]
struct NewInboundMessage {
    from_pubkey: Vec<u8>,
    message_type: String,
    message_json: String,
}

#[derive(Debug, Clone)]
pub struct SqliteMessageLog {
    path: PathBuf,
}

impl SqliteMessageLog {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path: if path.is_dir() {
                path.join("message_log.sqlite")
            } else {
                path
            },
        }
    }

    pub fn log_outbound_message<T: Serialize, V: Into<Vec<u8>>>(
        &self,
        destination_type: &str,
        destination: V,
        message_type: &str,
        message: &T,
    ) {
        if let Some(conn) = self.connect() {
            let _ = diesel::insert_into(outbound_messages::table)
                .values(NewOutboundMessage {
                    destination_type: destination_type.to_string(),
                    destination_pubkey: destination.into(),
                    message_type: message_type.to_string(),
                    message_json: serde_json::to_string(message).unwrap(),
                })
                .execute(&conn)
                .map_err(|e| {
                    error!(target: LOG_TARGET, "Failed to log outbound message: {}", e);
                });
        } else {
            error!(target: LOG_TARGET, "Could not connect to database to log message");
        }
    }

    pub fn log_inbound_message<T: Serialize, V: Into<Vec<u8>>>(&self, from_peer: V, message_type: &str, message: &T) {
        if let Some(conn) = self.connect() {
            let _ = diesel::insert_into(inbound_messages::table)
                .values(NewInboundMessage {
                    from_pubkey: from_peer.into(),
                    message_type: message_type.to_string(),
                    message_json: serde_json::to_string(message).unwrap(),
                })
                .execute(&conn)
                .map_err(|e| {
                    error!(target: LOG_TARGET, "Failed to log inbound message: {}", e);
                });
        } else {
            error!(target: LOG_TARGET, "Could not connect to database to log message");
        }
    }

    pub fn connect(&self) -> Option<SqliteConnection> {
        let database_url = &self.path;

        let _ = create_dir_all(database_url.parent().unwrap()).map_err(|e| {
            error!(
                target: LOG_TARGET,
                "Could not create message_logging_dir directory: {}", e
            )
        });

        let database_url = database_url.to_str().expect("database_url utf-8 error").to_string();
        if let Ok(connection) = SqliteConnection::establish(&database_url)
            .map_err(|e| error!(target: LOG_TARGET, "Could not connect to message_log database: {}", e))
        {
            embed_migrations!("./migrations");
            if let Err(err) = embedded_migrations::run_with_output(&connection, &mut std::io::stdout()) {
                log::error!(target: LOG_TARGET, "Error running migrations: {}", err);
            }
            Some(connection)
        } else {
            None
        }
    }
}
