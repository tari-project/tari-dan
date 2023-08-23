//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

pub fn setup_logger() {
    let _ignore = fern::Dispatch::new()
        // Perform allocation-free log formatting
        .format(|out, message, record| {
            out.finish(format_args!(
                "{} [{}] {} {}",
                humantime::format_rfc3339(std::time::SystemTime::now()),
                record.target(),
                record.level(),
                message
            ))
        })
        // Add blanket level filter -
        .level(log::LevelFilter::Debug)
        // Output to stdout, files, and other Dispatch configurations
        .chain(std::io::stdout())
        // .chain(fern::log_file("output.log").unwrap())
        // Apply globally
        .apply();
}
