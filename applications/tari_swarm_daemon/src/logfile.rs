//   Copyright 2024 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Bounded, mmap-backed reads over process log files.
//!
//! Swarm log files grow without bound, so nothing here ever reads a whole file: callers ask for a byte window and
//! get back the lines fully contained in it.

use std::{fs::File, io, path::Path};

use memmap2::{Mmap, MmapOptions};

/// Window size used when a caller does not ask for one.
pub const DEFAULT_CHUNK_BYTES: u64 = 512 * 1024;
/// Ceiling on a single window, so one request cannot pull a multi-gigabyte log into memory.
pub const MAX_CHUNK_BYTES: u64 = 4 * 1024 * 1024;
/// How far back [`tail_lines`] is willing to look for its lines.
const TAIL_SCAN_BYTES: u64 = 1024 * 1024;

/// A contiguous byte window of a log file, trimmed to whole lines.
#[derive(Debug)]
pub struct LogChunk {
    pub contents: String,
    /// Offset of the first byte returned. `0` means the window reaches the start of the file.
    pub start: u64,
    /// Offset one past the last byte returned. Ask for a window ending here to get the preceding lines.
    pub end: u64,
    /// Size of the file at the time of the read, so a caller can tell whether more has been appended since.
    pub file_size: u64,
}

/// Reads at most `max_bytes` ending at `end`, defaulting to the tail of the file.
///
/// The returned window starts on a line boundary: unless it reaches offset 0 the leading partial line is dropped and
/// `start` moves past it. `end` is returned unchanged, so paging backwards - passing the previous `start` as the next
/// `end` - neither drops nor repeats a line.
pub fn read_chunk(path: &Path, end: Option<u64>, max_bytes: Option<u64>) -> io::Result<LogChunk> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();

    let end = end.unwrap_or(file_size).min(file_size);
    let max_bytes = max_bytes.unwrap_or(DEFAULT_CHUNK_BYTES).clamp(1, MAX_CHUNK_BYTES);
    let mut start = end.saturating_sub(max_bytes);

    if start == end {
        return Ok(LogChunk {
            contents: String::new(),
            start,
            end,
            file_size,
        });
    }

    let window = map_window(&file, start, end)?;
    let mut bytes = &window[..];

    // A window that does not begin at the start of the file almost certainly begins mid-line.
    if start > 0 {
        match bytes.iter().position(|b| *b == b'\n') {
            Some(nl) => {
                bytes = &bytes[nl + 1..];
                start += nl as u64 + 1;
            },
            None => {
                bytes = &[];
                start = end;
            },
        }
    }

    Ok(LogChunk {
        contents: String::from_utf8_lossy(bytes).into_owned(),
        start,
        end,
        file_size,
    })
}

/// Returns up to the last `n` lines of the file, scanning back at most [`TAIL_SCAN_BYTES`].
pub fn tail_lines(path: &Path, n: usize) -> io::Result<Vec<String>> {
    let file = File::open(path)?;
    let file_size = file.metadata()?.len();
    let start = file_size.saturating_sub(TAIL_SCAN_BYTES);
    if start == file_size {
        return Ok(Vec::new());
    }

    let window = map_window(&file, start, file_size)?;
    let text = String::from_utf8_lossy(&window);
    let mut lines = text
        .lines()
        .rev()
        .take(n)
        .map(|line| line.to_string())
        .collect::<Vec<_>>();
    lines.reverse();

    // The first line of a window that started mid-file is a fragment, and is only worth keeping if it is all we have.
    if start > 0 && lines.len() > 1 {
        lines.remove(0);
    }

    Ok(lines)
}

fn map_window(file: &File, start: u64, end: u64) -> io::Result<Mmap> {
    let len = usize::try_from(end - start).map_err(|_| io::Error::other("log window exceeds addressable memory"))?;
    // SAFETY: the mapping is read-only and is dropped before the caller returns. Swarm rotates logs by rename and
    // otherwise only appends, so a mapped region is never truncated underneath us.
    unsafe { MmapOptions::new().offset(start).len(len).map(file) }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn write_temp(name: &str, contents: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("swarm-logfile-{name}.log"));
        let mut file = File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn empty_file_yields_an_empty_chunk() {
        let path = write_temp("empty", "");
        let chunk = read_chunk(&path, None, None).unwrap();
        assert_eq!(chunk.contents, "");
        assert_eq!(chunk.file_size, 0);
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, 0);
    }

    #[test]
    fn a_window_reaching_the_start_keeps_the_first_line() {
        let path = write_temp("whole", "one\ntwo\nthree\n");
        let chunk = read_chunk(&path, None, None).unwrap();
        assert_eq!(chunk.contents, "one\ntwo\nthree\n");
        assert_eq!(chunk.start, 0);
        assert_eq!(chunk.end, 14);
    }

    #[test]
    fn a_partial_leading_line_is_dropped() {
        let path = write_temp("partial", "one\ntwo\nthree\n");
        let chunk = read_chunk(&path, None, Some(9)).unwrap();
        assert_eq!(chunk.contents, "three\n");
        assert_eq!(chunk.start, 8);
    }

    #[test]
    fn paging_backwards_covers_every_line_exactly_once() {
        let path = write_temp("paging", "one\ntwo\nthree\n");
        let tail = read_chunk(&path, None, Some(9)).unwrap();
        let older = read_chunk(&path, Some(tail.start), Some(9)).unwrap();
        assert_eq!(older.contents, "one\ntwo\n");
        assert_eq!(older.start, 0);
    }

    #[test]
    fn paging_a_multi_megabyte_file_reconstructs_it_exactly() {
        let body = (0..40_000)
            .map(|i| format!("2026-09-07 10:58:49.9563 [tari::ootle] INFO line {i} 🌐 padded out a little\n"))
            .collect::<String>();
        let path = write_temp("large", &body);

        let mut pages = Vec::new();
        let mut end = None;
        loop {
            let chunk = read_chunk(&path, end, Some(256 * 1024)).unwrap();
            assert_eq!(chunk.file_size, body.len() as u64);
            pages.push(chunk.contents);
            if chunk.start == 0 {
                break;
            }
            end = Some(chunk.start);
        }

        assert!(
            pages.len() > 10,
            "expected the file to span many pages, got {}",
            pages.len()
        );
        pages.reverse();
        assert_eq!(pages.concat(), body);
    }

    #[test]
    fn a_window_never_splits_a_multibyte_character() {
        // Chosen so that the window boundary lands inside the four-byte emoji rather than between lines.
        let body = "aaaa\n🌐🌐🌐\nbbbb\n";
        let path = write_temp("utf8", body);
        for max in 1..=body.len() as u64 {
            let chunk = read_chunk(&path, None, Some(max)).unwrap();
            assert!(
                body.ends_with(&chunk.contents),
                "max={max} returned {:?}, which is not a suffix of the file",
                chunk.contents
            );
        }
    }

    #[test]
    fn tail_lines_returns_the_last_lines_in_order() {
        let path = write_temp("tail", "one\ntwo\nthree\n");
        assert_eq!(tail_lines(&path, 2).unwrap(), vec!["two", "three"]);
        assert_eq!(tail_lines(&path, 50).unwrap(), vec!["one", "two", "three"]);
    }
}
