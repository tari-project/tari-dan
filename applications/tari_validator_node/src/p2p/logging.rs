//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use serde::Serialize;
use sqlite_message_logger::SqliteMessageLogger;

pub trait MessageLogger {
    fn log_outbound_message<T: Serialize>(
        &self,
        destination_type: &str,
        destination: &str,
        message_type: &str,
        message_tag: &str,
        message: &T,
    );

    fn log_inbound_message<T: Serialize>(&self, source: &str, message_type: &str, message_tag: &str, message: &T);
}

impl MessageLogger for SqliteMessageLogger {
    fn log_outbound_message<T: Serialize>(
        &self,
        destination_type: &str,
        destination: &str,
        message_type: &str,
        message_tag: &str,
        message: &T,
    ) {
        self.log_outbound_message(destination_type, destination, message_type, message_tag, message)
    }

    fn log_inbound_message<T: Serialize>(&self, source: &str, message_type: &str, message_tag: &str, message: &T) {
        self.log_inbound_message(source, message_type, message_tag, message)
    }
}
