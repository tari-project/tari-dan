//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{fmt, fmt::Display, str::FromStr};

use crate::{TariNetwork, TariSwarmError};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtocolVersion<'a> {
    domain: &'a str,
    network: TariNetwork,
    version: Version,
}

impl<'a> ProtocolVersion<'a> {
    pub const fn new(domain: &'a str, network: TariNetwork, version: Version) -> Self {
        Self {
            domain,
            network,
            version,
        }
    }

    pub const fn domain(&self) -> &'a str {
        self.domain
    }

    pub const fn network(&self) -> TariNetwork {
        self.network
    }

    pub const fn version(&self) -> Version {
        self.version
    }

    pub fn is_compatible(&self, other: &ProtocolVersion) -> bool {
        self.domain == other.domain && self.network == other.network && self.version.semantic_version_eq(&other.version)
    }
}

impl PartialEq<String> for ProtocolVersion<'_> {
    fn eq(&self, other: &String) -> bool {
        let mut parts = other.split('/');
        let Some(domain) = parts.next() else {
            return false;
        };

        let Some(network) = parts.next() else {
            return false;
        };

        let Some(version) = parts.next().and_then(|s| s.parse().ok()) else {
            return false;
        };

        self.domain == domain && self.network.as_str() == network && self.version == version
    }
}

impl<'a> TryFrom<&'a str> for ProtocolVersion<'a> {
    type Error = TariSwarmError;

    fn try_from(value: &'a str) -> Result<Self, Self::Error> {
        let mut parts = value.split('/');
        // Must have a leading '/'
        let leading = parts.next();
        if leading.filter(|l| l.is_empty()).is_none() {
            return Err(TariSwarmError::ProtocolVersionParseFailed { field: "leading '/'" });
        }

        let mut next = move |field| parts.next().ok_or(TariSwarmError::ProtocolVersionParseFailed { field });
        Ok(Self::new(
            next("domain")?,
            next("network")?.parse()?,
            next("version")?.parse()?,
        ))
    }
}

impl Display for ProtocolVersion<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "/{}/{}/{}", self.domain, self.network, self.version)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Version {
    major: u16,
    minor: u16,
    patch: u16,
}

impl Version {
    pub const fn new(major: u16, minor: u16, patch: u16) -> Self {
        Self { major, minor, patch }
    }

    pub const fn major(&self) -> u16 {
        self.major
    }

    pub const fn minor(&self) -> u16 {
        self.minor
    }

    pub const fn patch(&self) -> u16 {
        self.patch
    }

    pub const fn semantic_version_eq(&self, other: &Version) -> bool {
        // Similar to https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#specifying-dependencies-from-cratesio
        // 0.x.y any change to x is not compatible
        if self.major == 0 {
            // 0.0.x any change to x is not compatible
            if self.minor == 0 {
                return self.patch == other.patch;
            }
            return self.minor == other.minor;
        }

        // x.y.z any change to x is not compatible
        self.major == other.major
    }
}

impl FromStr for Version {
    type Err = TariSwarmError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut parts = s.split('.');

        let mut next = move |field| {
            parts
                .next()
                .ok_or(TariSwarmError::ProtocolVersionParseFailed { field })?
                .parse()
                .map_err(|_| TariSwarmError::ProtocolVersionParseFailed { field })
        };
        Ok(Self {
            major: next("version.major")?,
            minor: next("version.minor")?,
            patch: next("version.patch")?,
        })
    }
}

impl Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn it_parses_correctly() {
        let version = ProtocolVersion::try_from("/tari/devnet/0.0.1").unwrap();
        assert_eq!(version.domain(), "tari");
        assert_eq!(version.network(), TariNetwork::DevNet);
        assert_eq!(version.version(), Version {
            major: 0,
            minor: 0,
            patch: 1
        });
    }
}
