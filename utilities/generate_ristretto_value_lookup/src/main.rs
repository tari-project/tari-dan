//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fs,
    io,
    io::{stdout, Write},
    time::Instant,
};

use human_bytes::human_bytes;
use tari_crypto::{
    keys::PublicKey,
    ristretto::{RistrettoPublicKey, RistrettoSecretKey},
    tari_utilities::ByteArray,
};

use crate::cli::Cli;
mod cli;

fn main() -> io::Result<()> {
    let cli = Cli::init();
    let dest_file = cli.output_file;

    let file_size = (cli.max - cli.min + 1) * 32 + 20;
    println!(
        "Generating Ristretto value lookup table from {} to {} and writing to {} ({})",
        cli.min,
        cli.max,
        dest_file.display(),
        human_bytes(file_size as f64)
    );

    println!();

    let writer = fs::File::create(&dest_file)?;

    let timer = Instant::now();
    write_output(writer, cli.min, cli.max)?;
    let elapsed = timer.elapsed();

    println!();

    let metadata = fs::metadata(&dest_file)?;

    println!(
        "Output written to {} ({}) in {:.2?}",
        dest_file.display(),
        human_bytes(metadata.len() as f64),
        elapsed
    );

    Ok(())
}

fn write_output<W: io::Write>(mut writer: W, min: u64, max: u64) -> io::Result<()> {
    // Write header VLKP || min_value (8 bytes) || max_value (8 bytes)
    writer.write_all(b"VLKP")?;
    writer.write_all(&min.to_be_bytes())?;
    writer.write_all(&max.to_be_bytes())?;

    let mut dot_count = 0;
    for v in min..=max {
        let p = RistrettoPublicKey::from_secret_key(&RistrettoSecretKey::from(v));
        writer.write_all(p.as_bytes())?;
        if v % 10000 == 0 {
            dot_count += 1;
            print!(".");
            stdout().flush()?;
        }
        if dot_count == 80 {
            dot_count = 0;
            println!();
        }
    }
    Ok(())
}
