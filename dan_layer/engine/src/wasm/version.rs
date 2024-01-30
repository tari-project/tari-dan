//  Copyright 2024. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

use std::{num::NonZeroU64, str::FromStr};

use semver::Version;

/// Versions are considered compatible if their left-most non-zero major/minor/patch component is the same
/// See https://doc.rust-lang.org/cargo/reference/resolver.html
#[derive(Clone, Copy, Eq, PartialEq)]
enum SemverCompatibility {
    Major(NonZeroU64),
    Minor(NonZeroU64),
    Patch(u64),
}

impl From<Version> for SemverCompatibility {
    fn from(ver: Version) -> Self {
        if let Some(m) = NonZeroU64::new(ver.major) {
            return SemverCompatibility::Major(m);
        }
        if let Some(m) = NonZeroU64::new(ver.minor) {
            return SemverCompatibility::Minor(m);
        }
        SemverCompatibility::Patch(ver.patch)
    }
}

impl FromStr for SemverCompatibility {
    type Err = semver::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let version = Version::parse(s)?;
        Ok(version.into())
    }
}

pub fn are_versions_compatible(a: &str, b: &str) -> Result<bool, semver::Error> {
    let a_compat = SemverCompatibility::from_str(a)?;
    let b_compat = SemverCompatibility::from_str(b)?;
    Ok(a_compat == b_compat)
}

#[cfg(test)]
mod tests {
    use crate::wasm::version::are_versions_compatible;

    #[test]
    fn it_accepts_compatible_versions() {
        assert!(are_versions_compatible("0.1.0", "0.1.2").unwrap());
        assert!(are_versions_compatible("1.0.3", "1.1.0").unwrap());
        assert!(are_versions_compatible("1.0.0-alpha.0", "1.0.0-alpha.1").unwrap());
    }

    #[test]
    fn it_rejects_incompatible_versions() {
        assert!(!are_versions_compatible("0.0.1", "0.0.2").unwrap());
        assert!(!are_versions_compatible("0.1.0", "0.2.0").unwrap());
        assert!(!are_versions_compatible("1.1.0", "2.0.0").unwrap());
    }
}
