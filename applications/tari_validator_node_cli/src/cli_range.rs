//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{ops::RangeInclusive, str::FromStr};

use tari_dan_common_types::Epoch;

#[derive(Debug, Clone)]
pub struct CliRange<T>(RangeInclusive<T>);

impl<T> CliRange<T> {
    pub fn into_inner(self) -> RangeInclusive<T> {
        self.0
    }
}

impl FromStr for CliRange<Epoch> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let range = CliRange::<u64>::from_str(s)?.into_inner();
        Ok(Self(RangeInclusive::new(Epoch(*range.start()), Epoch(*range.end()))))
    }
}

impl FromStr for CliRange<u64> {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let parsed = parse_range(s)?;
        let start = parsed.start.map(u64::from_str).transpose()?.unwrap_or(0);
        let end = parsed.end.map(u64::from_str).transpose()?.unwrap_or(u64::MAX);
        if parsed.inclusive {
            Ok(Self(RangeInclusive::new(start, end)))
        } else {
            Ok(Self(RangeInclusive::new(start, end.saturating_sub(1))))
        }
    }
}

fn parse_range(s: &str) -> Result<ParsedRange<'_>, anyhow::Error> {
    let Some((start, end)) = s.split_once("..") else {
        // If the user enters just a value, we treat it as a V..=V range
        return Ok(ParsedRange {
            start: Some(s),
            end: Some(s),
            inclusive: true,
        });
    };

    let inclusive = end.starts_with('=');
    let end = if inclusive { &end[1..] } else { end };
    Ok(ParsedRange {
        start: if start.is_empty() { None } else { Some(start) },
        end: if end.is_empty() { None } else { Some(end) },
        inclusive,
    })
}

struct ParsedRange<'a> {
    start: Option<&'a str>,
    end: Option<&'a str>,
    inclusive: bool,
}
