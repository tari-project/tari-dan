//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use std::{fs, path::PathBuf};

use fern::FormatCallback;

/// Target used by [`crate::process_manager`] when it relays a child process's own output.
pub const FORWARDED_TARGET: &str = "swarm";

pub fn init_logger(log_to_file: Option<PathBuf>) -> Result<(), log::SetLoggerError> {
    fn should_skip(target: &str) -> bool {
        const SKIP: &[&str] = &[
            "hyper",
            "h2",
            "tower",
            "hyper_util",
            "tokio",
            "tokio_util",
            "cranelift_codegen",
        ];
        target.is_empty() || SKIP.iter().any(|s| target.starts_with(s))
    }

    let colors = fern::colors::ColoredLevelConfig::new().info(fern::colors::Color::Green);
    let mut logger = fern::Dispatch::new()
        .format(move |out, message, record| {
            if should_skip(record.target()) {
                return;
            }

            let fallback = |out: FormatCallback<'_>| {
                out.finish(format_args!(
                    "{} [{}] {} {}",
                    humantime::format_rfc3339(std::time::SystemTime::now()),
                    record.target(),
                    colors.color(record.level()),
                    message
                ))
            };

            // Only forwarded child output embeds the process's own target and level in the message text, and only it
            // is guaranteed to be a single line. Every other record is formatted as written.
            if record.target() != FORWARDED_TARGET {
                fallback(out);
                return;
            }

            // Example: [Validator node-#1] 12:55 INFO Received vote for block #NodeHeight(88)
            // d9abc7b1bb66fd912848f5bc4e5a69376571237e3243dc7f6a91db02bb5cf37c from
            // a08cf5038e8e3cda8e3716c79f769cd42fad05f7110628efb5be6a40e28bc94c (4 of 3) Implement a naive
            // parsing of the log message to extract the target, level and the log message from each running process
            let message_str = message.to_string();
            let Some((target, rest)) = message_str.split_once(']') else {
                fallback(out);
                return;
            };

            let mut parts = rest.trim().splitn(3, ' ');

            // Skip the time
            if parts.next().is_none() {
                fallback(out);
                return;
            }

            let Some(level) = parts.next().and_then(|s| s.parse().ok()).map(|l| colors.color(l)) else {
                fallback(out);
                return;
            };

            let Some(log) = parts.next() else {
                fallback(out);
                return;
            };

            out.finish(format_args!(
                "{} {}] {} {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                target,
                level,
                log
            ))
        })
        .chain(
            fern::Dispatch::new()
                .level(log::LevelFilter::Info)
                .chain(std::io::stdout()),
        );
    if let Some(log) = log_to_file {
        logger = logger.chain(
            fern::Dispatch::new().level(log::LevelFilter::Debug).chain(
                fs::OpenOptions::new()
                    .create(true)
                    .write(true)
                    // Files get massive, so we truncate on each start
                    .truncate(true)
                    .open(log.join("swarm.log"))
                    .expect("Failed to open log file"),
            ),
        );
    }
    logger.apply()
}
