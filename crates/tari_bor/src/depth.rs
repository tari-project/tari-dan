//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

//! Structural scans of untrusted input that run before any typed decode: the codec-wide nesting
//! bound, and the item count a decode's cost is priced by.

#[cfg(not(feature = "std"))]
use alloc::vec::Vec;

use minicbor::{Decoder, data::Type, decode};

/// Rejects input nested deeper than `max_depth`.
///
/// The walk is iterative and reads only item heads, so it costs a fraction of the decode it guards
/// and is itself immune to the recursion it rejects.
pub fn check_nesting_depth(input: &[u8], max_depth: usize) -> Result<(), decode::Error> {
    // Malformed input passes: the decode that follows produces a parse error far more specific than
    // anything this walk could say.
    if !matches!(walk(&mut Decoder::new(input), max_depth), Ok(None)) {
        return Ok(());
    }
    Err(decode::Error::message("maximum CBOR nesting depth exceeded"))
}

/// Counts the data items making up the first item in `input`: that item and every item nested in
/// it. Each chunk of an indefinite-length byte or text string counts as an item of its own.
///
/// Decoding into a [`crate::Value`] does a roughly fixed amount of work per item on top of copying
/// its bytes, so this count, rather than the input's length, is what prices that decode. Like
/// [`check_nesting_depth`] it is iterative and reads only item heads.
pub fn count_data_items(input: &[u8]) -> Result<u64, decode::Error> {
    // A walk bounded at `usize::MAX` is never cut short for depth.
    Ok(walk(&mut Decoder::new(input), usize::MAX)?.unwrap_or(u64::MAX))
}

/// Counts the data items making up the first item in `input`, as [`count_data_items`] does, and
/// rejects it as [`check_nesting_depth`] does: one walk that both prices a decode and guards it.
///
/// Unlike [`check_nesting_depth`], input the walk cannot read fails here, since there is no count to
/// give for it.
pub fn count_data_items_with_max_depth(input: &[u8], max_depth: usize) -> Result<u64, decode::Error> {
    walk(&mut Decoder::new(input), max_depth)?
        .ok_or_else(|| decode::Error::message("maximum CBOR nesting depth exceeded"))
}

/// Walks the heads of the first item in `d`, returning how many items it holds, or `None` once an
/// item nests deeper than `max_depth`.
fn walk(d: &mut Decoder<'_>, max_depth: usize) -> Result<Option<u64>, decode::Error> {
    // Items still to read per open container, innermost last: `Some(n)` for a definite-length
    // container, `None` for an indefinite-length one that ends at a break byte. The outermost frame
    // is the single top-level item, so an item's nesting depth is `stack.len() - 1`.
    let mut stack: Vec<Option<u64>> = Vec::with_capacity(16);
    stack.push(Some(1));
    let mut items = 0u64;

    loop {
        let Some(frame) = stack.last().copied() else {
            return Ok(Some(items));
        };
        match frame {
            Some(0) => {
                stack.pop();
                continue;
            },
            Some(_) => {},
            None => {
                if matches!(d.datatype()?, Type::Break) {
                    d.skip()?;
                    stack.pop();
                    continue;
                }
            },
        }

        // An item is about to be read, and the frames open around it are its nesting. Checked here
        // rather than where a container is opened, so an empty container at the bound is accepted
        // exactly as a scalar there is.
        if stack.len() - 1 > max_depth {
            return Ok(None);
        }

        if let Some(Some(n)) = stack.last_mut() {
            *n -= 1;
        }
        items = items.saturating_add(1);

        let remaining = match d.datatype()? {
            Type::Array | Type::ArrayIndef => d.array()?,
            // A map's key and value each nest at the same depth, so one frame covers both.
            Type::Map | Type::MapIndef => d.map()?.map(|n| n.saturating_mul(2)),
            Type::Tag => {
                d.tag()?;
                Some(1)
            },
            // An indefinite-length string's chunks do not nest, but each is decoded separately.
            Type::BytesIndef => {
                for chunk in d.bytes_iter()? {
                    chunk?;
                    items = items.saturating_add(1);
                }
                continue;
            },
            Type::StringIndef => {
                for chunk in d.str_iter()? {
                    chunk?;
                    items = items.saturating_add(1);
                }
                continue;
            },
            _ => {
                d.skip()?;
                continue;
            },
        };

        stack.push(remaining);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Any bound will do for exercising the walk; the real one is the caller's.
    const MAX: usize = 256;

    #[test]
    fn scalars_and_flat_containers_pass() {
        for input in [
            &[0x00][..],                                                 // 0
            &[0x3b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff][..], // a 64-bit negative
            &[0x80][..],                                                 // []
            &[0xa0][..],                                                 // {}
            &[0x83, 0x01, 0x02, 0x03][..],                               // [1, 2, 3]
            &[0xa1, 0x01, 0x02][..],                                     // {1: 2}
            &[0x9f, 0x01, 0xff][..],                                     // [_ 1]
            &[0xbf, 0x01, 0x02, 0xff][..],                               // {_ 1: 2}
            &[0x43, 0x01, 0x02, 0x03][..],                               // h'010203'
            &[0x5f, 0x41, 0x01, 0x41, 0x02, 0xff][..],                   // (_ h'01', h'02')
            &[0xc0, 0x01][..],                                           // 0(1)
        ] {
            assert!(check_nesting_depth(input, MAX).is_ok(), "rejected {input:x?}");
        }
    }

    #[test]
    fn trailing_bytes_after_the_first_item_are_not_walked() {
        // A flat item followed by a nesting bomb: only the first item is this walk's business.
        let mut input = vec![0x00];
        input.extend(core::iter::repeat_n(0x81u8, 10_000));
        assert!(check_nesting_depth(&input, MAX).is_ok());
    }

    #[test]
    fn nesting_at_the_bound_passes_and_one_level_deeper_is_rejected() {
        for head in [
            0x81u8, // array of 1
            0xa1,   // map of 1
            0x9f,   // indefinite array
            0xc0,   // tag
        ] {
            let at_bound: Vec<u8> = core::iter::repeat_n(head, MAX).chain(core::iter::once(0x00)).collect();
            assert!(check_nesting_depth(&at_bound, MAX).is_ok(), "head {head:#x} at bound");

            let over_bound: Vec<u8> = core::iter::repeat_n(head, MAX + 1)
                .chain(core::iter::once(0x00))
                .collect();
            assert!(
                check_nesting_depth(&over_bound, MAX).is_err(),
                "head {head:#x} over bound"
            );
        }
    }

    #[test]
    fn an_empty_container_at_the_bound_is_accepted_as_a_scalar_there_is() {
        let containers: Vec<u8> = core::iter::repeat_n(0x81u8, MAX).collect();
        for innermost in [0x00u8, 0x80, 0xa0] {
            let mut input = containers.clone();
            input.push(innermost);
            assert!(check_nesting_depth(&input, MAX).is_ok(), "innermost {innermost:#x}");
        }
    }

    #[test]
    fn a_nesting_bomb_is_rejected_without_recursing() {
        let bomb = vec![0x81u8; 1_000_000];
        assert!(check_nesting_depth(&bomb, MAX).is_err());
    }

    #[test]
    fn malformed_input_is_left_for_the_decoder_to_reject() {
        for input in [
            &[][..],     // no input at all
            &[0x81][..], // array of 1 with no element
            &[0xff][..], // a stray break
            &[0x1c][..], // a reserved additional-information value
        ] {
            assert!(check_nesting_depth(input, MAX).is_ok(), "claimed {input:x?} for itself");
        }
    }

    #[test]
    fn every_nested_item_is_counted() {
        for (input, expected) in [
            (&[0x00][..], 1),                               // 0
            (&[0x80][..], 1),                               // []
            (&[0x83, 0x01, 0x02, 0x03][..], 4),             // [1, 2, 3]
            (&[0xa1, 0x01, 0x02][..], 3),                   // {1: 2}
            (&[0x9f, 0x01, 0xff][..], 2),                   // [_ 1]
            (&[0xbf, 0x01, 0x02, 0xff][..], 3),             // {_ 1: 2}
            (&[0x43, 0x01, 0x02, 0x03][..], 1),             // h'010203'
            (&[0x5f, 0x41, 0x01, 0x41, 0x02, 0xff][..], 3), // (_ h'01', h'02')
            (&[0x7f, 0x61, 0x61, 0x60, 0xff][..], 3),       // (_ "a", "")
            (&[0xc0, 0x01][..], 2),                         // 0(1)
            (&[0x82, 0x81, 0x00, 0xa1, 0x00, 0x80][..], 6), // [[0], {0: []}]
            (&[0x00, 0x83, 0x01, 0x02, 0x03][..], 1),       // 0, then trailing bytes
        ] {
            assert_eq!(count_data_items(input).unwrap(), expected, "counted {input:x?}");
        }
    }

    #[test]
    fn counting_is_not_bounded_by_nesting() {
        let mut deep = vec![0x81u8; 10_000];
        deep.push(0x00);
        assert_eq!(count_data_items(&deep).unwrap(), 10_001);
    }

    #[test]
    fn a_bounded_count_counts_within_the_bound_and_rejects_beyond_it() {
        let mut at_bound = vec![0x81u8; MAX];
        at_bound.push(0x00);
        assert_eq!(count_data_items_with_max_depth(&at_bound, MAX).unwrap(), MAX as u64 + 1);

        let mut over_bound = vec![0x81u8; MAX + 1];
        over_bound.push(0x00);
        assert!(count_data_items_with_max_depth(&over_bound, MAX).is_err());

        for input in [&[][..], &[0x81][..], &[0x1c][..]] {
            assert!(
                count_data_items_with_max_depth(input, MAX).is_err(),
                "counted {input:x?}"
            );
        }
    }

    #[test]
    fn counting_malformed_input_fails() {
        for input in [&[][..], &[0x81][..], &[0x1c][..]] {
            assert!(count_data_items(input).is_err(), "counted {input:x?}");
        }
    }
}
