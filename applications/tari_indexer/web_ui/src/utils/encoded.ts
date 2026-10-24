//  Copyright 2026 The Tari Project
//  SPDX-License-Identifier: BSD-3-Clause

import type { ExecuteResult, SubstateValue } from "@tari-project/ootle-ts-bindings";

// The indexer serves engine values in their CBOR encoding, base64 in JSON. Reading one needs a decoder that knows the
// engine's CBOR schema. This UI has none, so each decoder returns `undefined` and callers show the encoded value.

export function decodeSubstateValue(_encoded: string): SubstateValue | undefined {
  return undefined;
}

export function decodeExecuteResult(_encoded: string): ExecuteResult | undefined {
  return undefined;
}

/** The number of bytes `encoded` (base64) holds. */
export function encodedByteLength(encoded: string): number {
  const padding = encoded.endsWith("==") ? 2 : encoded.endsWith("=") ? 1 : 0;
  return Math.floor((encoded.length * 3) / 4) - padding;
}
