//    Copyright 2024 The Tari Project
//    SPDX-License-Identifier: BSD-3-Clause

use fern::FormatCallback;

pub fn init_logger() -> Result<(), log::SetLoggerError> {
    fn should_skip(target: &str) -> bool {
        const SKIP: [&str; 3] = ["hyper::", "h2::", "tower::"];
        if SKIP.iter().any(|s| target.starts_with(s)) {
            return true;
        }

        false
    }

    let colors = fern::colors::ColoredLevelConfig::new()
        .info(fern::colors::Color::Green)
        .debug(fern::colors::Color::Yellow)
        .error(fern::colors::Color::Red);
    fern::Dispatch::new()
        .format(move |out, message, record| {
            if should_skip(record.target()) {
                return;
            }

            let fallback =   |out: FormatCallback<'_>|  out.finish(format_args!(
                "[{} {} {}] {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                record.metadata().target(),
                colors.color(record.level()),
                message
            ));

            // Example: [Validator node-#1] 12:55 INFO Received vote for block #NodeHeight(88) d9abc7b1bb66fd912848f5bc4e5a69376571237e3243dc7f6a91db02bb5cf37c from a08cf5038e8e3cda8e3716c79f769cd42fad05f7110628efb5be6a40e28bc94c (4 of 3)
            // Implement a naive parsing of the log message to extract the target, level and the log message from each running process
            let message_str = message.to_string();
            let Some((target, rest)) = message_str.split_once( ']') else {
                fallback(out);
                return;
            };

            let mut parts = rest.trim().splitn(3, ' ');

            // Skip the time
            if parts.next().is_none() {
                fallback(out);
                return;
            }

            let Some(level) = parts.next()
                .and_then(|s| s.parse().ok())
                .map(|l| colors.color(l)) else {
                fallback(out);
                return;
            };

            let Some(log) = parts.next() else {
                fallback(out);
                return;
            };

            out.finish(format_args!(
                "{} {} {}] {} {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                record.metadata().target(),
                target,
                level,
                log
            ))
        })
        .filter(|record_metadata| record_metadata.target().starts_with("tari_watcher")) // skip tokio frame prints
        .level(log::LevelFilter::Debug)
        .chain(std::io::stdout())
        // .chain(fern::log_file("output.log").unwrap())
        .apply()
}
