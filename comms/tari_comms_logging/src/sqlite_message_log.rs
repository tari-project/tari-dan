//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Formatter},
    fs::create_dir_all,
    path::PathBuf,
    sync::{Arc, Mutex, MutexGuard},
};

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

#[derive(Clone)]
pub struct SqliteMessageLog {
    connection: Option<Arc<Mutex<SqliteConnection>>>,
}

impl SqliteMessageLog {
    pub fn new(mut path: PathBuf) -> Self {
        if path.is_dir() {
            path = path.join("message_log.sqlite");
        }

        let _ = create_dir_all(path.parent().unwrap()).map_err(|e| {
            error!(
                target: LOG_TARGET,
                "Could not create message_logging_dir directory: {}", e
            )
        });

        let path = path.to_str().expect("path utf-8 error").to_string();
        match SqliteConnection::establish(&path) {
            Ok(connection) => {
                embed_migrations!("./migrations");
                if let Err(err) = embedded_migrations::run_with_output(&connection, &mut std::io::stdout()) {
                    log::error!(target: LOG_TARGET, "Error running migrations: {}", err);
                }

                Self {
                    connection: Some(Arc::new(Mutex::new(connection))),
                }
            },
            Err(e) => {
                error!(target: LOG_TARGET, "Could not connect to message log database: {}", e);
                Self { connection: None }
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
                    message_json: serde_json::to_string_pretty(message).unwrap(),
                })
                .execute(&*conn)
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
                    message_json: serde_json::to_string_pretty(message).unwrap(),
                })
                .execute(&*conn)
                .map_err(|e| {
                    error!(target: LOG_TARGET, "Failed to log inbound message: {}", e);
                });
        } else {
            error!(target: LOG_TARGET, "Could not connect to database to log message");
        }
    }

    fn connect(&self) -> Option<MutexGuard<SqliteConnection>> {
        Some(self.connection.as_ref()?.lock().unwrap())
    }
}

impl Debug for SqliteMessageLog {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("SqliteMessageLog")
    }
}
