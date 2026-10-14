//   Copyright 2026 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_engine_types::ProtocolVersion;
use tari_jellyfish::JmtHashScheme;

/// The JMT node hash scheme a root committed under `protocol_version` was built with.
///
/// A verifier selects the scheme from its authenticated context (the protocol version of the header that commits the
/// root), never from a proof. The match lists every version so that a new protocol version must state its scheme.
pub fn jmt_hash_scheme(protocol_version: ProtocolVersion) -> JmtHashScheme {
    match protocol_version {
        ProtocolVersion::V0 | ProtocolVersion::V1 | ProtocolVersion::V2 => JmtHashScheme::V1,
    }
}
