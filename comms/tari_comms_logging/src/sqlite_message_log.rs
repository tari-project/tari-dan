//  Copyright 2022 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Display, Formatter},
    fs::create_dir_all,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

use chrono::NaiveDateTime;
use diesel::{prelude::*, sql_query};
use log::error;
use serde::{Deserialize, Serialize};

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
    message_tag: String,
}

#[derive(Debug, Insertable)]
#[table_name = "inbound_messages"]
struct NewInboundMessage {
    from_pubkey: Vec<u8>,
    message_type: String,
    message_json: String,
    message_tag: String,
}

#[derive(Debug, Clone, QueryableByName, Serialize, Deserialize)]
pub struct LoggedMessage {
    #[sql_type = "diesel::sql_types::Integer"]
    pub id: i32,
    #[sql_type = "diesel::sql_types::Text"]
    pub in_out: String,
    #[sql_type = "diesel::sql_types::Blob"]
    pub pubkey: Vec<u8>,
    #[sql_type = "diesel::sql_types::Text"]
    pub message_type: String,
    #[sql_type = "diesel::sql_types::Text"]
    pub message_json: String,
    #[sql_type = "diesel::sql_types::Timestamp"]
    pub timestamp: NaiveDateTime,
}

impl Display for LoggedMessage {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        writeln!(
            f,
            "==============================================================================="
        )?;
        writeln!(
            f,
            "ID: {}, Direction: {}, Peer: {}.., Timestamp: {}",
            self.id,
            self.in_out,
            self.pubkey
                .iter()
                .take(8)
                .map(|b| format!("{:02x}", b))
                .collect::<Vec<String>>()
                .join(""),
            self.timestamp
        )?;
        writeln!(
            f,
            "==============================================================================="
        )?;
        writeln!(f, "{}", self.message_json,)
    }
}

#[derive(Clone)]
pub struct SqliteMessageLog {
    connection: Option<Arc<Mutex<SqliteConnection>>>,
}

impl SqliteMessageLog {
    pub fn new<P: AsRef<Path>>(path: P) -> Self {
        let path = if path.as_ref().is_dir() {
            path.as_ref().join("message_log.sqlite")
        } else {
            path.as_ref().to_path_buf()
        };

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
        message_tag: String,
        message: &T,
    ) {
        if let Some(conn) = self.connect() {
            let _ = diesel::insert_into(outbound_messages::table)
                .values(NewOutboundMessage {
                    destination_type: destination_type.to_string(),
                    destination_pubkey: destination.into(),
                    message_type: message_type.to_string(),
                    message_json: serde_json::to_string_pretty(message).unwrap(),
                    message_tag,
                })
                .execute(&*conn)
                .map_err(|e| {
                    error!(target: LOG_TARGET, "Failed to log outbound message: {}", e);
                });
        } else {
            error!(target: LOG_TARGET, "Could not connect to database to log message");
        }
    }

    pub fn log_inbound_message<T: Serialize, V: Into<Vec<u8>>>(
        &self,
        from_peer: V,
        message_type: &str,
        message_tag: String,
        message: &T,
    ) {
        if let Some(conn) = self.connect() {
            let _ = diesel::insert_into(inbound_messages::table)
                .values(NewInboundMessage {
                    from_pubkey: from_peer.into(),
                    message_type: message_type.to_string(),
                    message_json: serde_json::to_string_pretty(message).unwrap(),
                    message_tag,
                })
                .execute(&*conn)
                .map_err(|e| {
                    error!(target: LOG_TARGET, "Failed to log inbound message: {}", e);
                });
        } else {
            error!(target: LOG_TARGET, "Could not connect to database to log message");
        }
    }

    pub fn get_messages_by_tag(&self, message_tag: String) -> Vec<LoggedMessage> {
        if let Some(conn) = self.connect() {
            sql_query(
                r#"
                SELECT
                    "Inbound" as in_out,
                    msg_in.id as id,
                    msg_in.from_pubkey as pubkey,
                    msg_in.message_type as message_type,
                    msg_in.message_json as message_json,
                    msg_in.received_at as timestamp
                FROM
                    inbound_messages msg_in
                WHERE msg_in.message_tag = ?
                UNION
                SELECT
                    "Outbound" as in_out,
                    msg_out.id as id,
                    msg_out.destination_pubkey as pubkey,
                    msg_out.message_type as message_type,
                    msg_out.message_json as message_json,
                    msg_out.sent_at as timestamp
                FROM
                    outbound_messages msg_out
                WHERE msg_out.message_tag = ?
                ORDER BY msg_out.sent_at ASC, msg_in.received_at ASC"#,
            )
            .bind::<diesel::sql_types::Text, _>(message_tag.clone())
            .bind::<diesel::sql_types::Text, _>(message_tag)
            .load::<LoggedMessage>(&*conn)
            .unwrap_or_else(|e| {
                error!(target: LOG_TARGET, "Failed to get messages by tag: {}", e);
                Vec::new()
            })
        } else {
            error!(target: LOG_TARGET, "Could not connect to database to log message");
            vec![]
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
