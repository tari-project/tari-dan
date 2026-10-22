//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

import { bech32m, Decoded } from "bech32";
import { Network } from "../types/Network";
import { OotleAddress } from "../types/OotleAddress";

const PAY_REF_MAX_LENGTH = 64;

// Longest HRP (8) + separator (1) + 64 key bytes and a full pay ref as 5-bit words + checksum (6).
const MAX_LENGTH_LIMIT = 8 + 1 + Math.ceil(((64 + PAY_REF_MAX_LENGTH) * 8) / 5) + 6;

export type DecodedOotleAddress = {
  network: Network;
  accountPublicKey: string;
  viewOnlyKey: string;
  payRef: string | Uint8Array | null;
};

export enum NetworkHrp {
  MainNet = "otl_",
  StageNet = "otl_stg_",
  NextNet = "otl_nxt_",
  LocalNet = "otl_loc_",
  Igor = "otl_igr_",
  Esmeralda = "otl_esm_",
}

export function decodeOotleAddressOrNull(address: OotleAddress): DecodedOotleAddress | null {
  try {
    return decodeOotleAddress(address);
  } catch (e) {
    console.warn("decodeOotleAddressOrNull: decode error:", e, address);
    return null;
  }
}

export function decodeOotleAddress(address: OotleAddress): DecodedOotleAddress {
  const { prefix, words }: Decoded = bech32m.decode(address, MAX_LENGTH_LIMIT);
  const data = bech32m.fromWords(words);

  if (data.length < 64) {
    throw new Error(`Invalid Ootle address length: ${data.length}, expected 64`);
  }

  const accountPublicKey = buf2hex(data.slice(0, 32));
  const viewOnlyKey = buf2hex(data.slice(32, 64));
  const payRefBytes = new Uint8Array(data.slice(64));

  // A pay ref that is valid UTF-8 is returned as a string, otherwise as raw bytes
  let payRef: string | Uint8Array | null = null;
  if (payRefBytes.length > 0) {
    try {
      payRef = new TextDecoder("utf-8", { fatal: true }).decode(payRefBytes);
    } catch (e) {
      payRef = payRefBytes;
    }
  }

  let network: Network;
  switch (prefix) {
    case NetworkHrp.MainNet:
      network = "mainnet";
      break;
    case NetworkHrp.StageNet:
      network = "stagenet";
      break;
    case NetworkHrp.NextNet:
      network = "nextnet";
      break;
    case NetworkHrp.LocalNet:
      network = "localnet";
      break;
    case NetworkHrp.Igor:
      network = "igor";
      break;
    case NetworkHrp.Esmeralda:
      network = "esmeralda";
      break;

    default:
      throw new Error("Invalid network prefix in Ootle address");
  }

  return {
    network,
    accountPublicKey,
    viewOnlyKey,
    payRef,
  };
}

export function encodeOotleAddress(decoded: DecodedOotleAddress): OotleAddress {
  let networkHrp: string;
  switch (decoded.network) {
    case "mainnet":
      networkHrp = NetworkHrp.MainNet;
      break;
    case "stagenet":
      networkHrp = NetworkHrp.StageNet;
      break;
    case "nextnet":
      networkHrp = NetworkHrp.NextNet;
      break;
    case "localnet":
      networkHrp = NetworkHrp.LocalNet;
      break;
    case "igor":
      networkHrp = NetworkHrp.Igor;
      break;
    case "esmeralda":
      networkHrp = NetworkHrp.Esmeralda;
      break;
    default:
      throw new Error(`Invalid network: ${decoded.network}`);
  }

  if (decoded.accountPublicKey.length !== 64 || decoded.viewOnlyKey.length !== 64) {
    throw new Error("Public keys must be 32 bytes (64 hex characters) long");
  }

  const accountPubKeyBytes = hex2buf(decoded.accountPublicKey);
  const viewOnlyKeyBytes = hex2buf(decoded.viewOnlyKey);
  const payRefBytes = decoded.payRef
    ? typeof decoded.payRef === "string"
      ? new TextEncoder().encode(decoded.payRef)
      : decoded.payRef
    : new Uint8Array();

  const addressBytes = new Uint8Array(accountPubKeyBytes.length + viewOnlyKeyBytes.length + payRefBytes.length);
  addressBytes.set(accountPubKeyBytes, 0);
  addressBytes.set(viewOnlyKeyBytes, accountPubKeyBytes.length);
  addressBytes.set(payRefBytes, accountPubKeyBytes.length + viewOnlyKeyBytes.length);
  const words = bech32m.toWords(addressBytes);
  return bech32m.encode(networkHrp, words, MAX_LENGTH_LIMIT) as OotleAddress;
}

const HEX = /[\da-fA-F]{2}/i;

function buf2hex(buffer: number[]) {
  return buffer.map((x) => x.toString(16).padStart(2, "0")).join("");
}

function hex2buf(string: string): Uint8Array {
  // Check if Uint8Array.fromHex is available (most modern browsers)
  const fromHex = (Uint8Array as any).fromHex;
  if (typeof fromHex === "function") {
    return fromHex(string);
  }

  const stringLength = string.length;
  if (stringLength % 2 !== 0) throw new SyntaxError("String should be an even number of characters");
  const maxLength = stringLength / 2;
  const bytes = new Uint8Array(maxLength);
  let read = 0;
  let written = 0;
  while (written < maxLength) {
    const hexits = string.slice(read, (read += 2));
    if (!HEX.exec(hexits)) throw new SyntaxError("String should only contain hex characters");
    bytes[written++] = parseInt(hexits, 16);
  }
  return bytes;
}

export function validateOotleAddress(address: string): boolean {
  if (!address || typeof address !== "string") {
    return false;
  }
  // Trim whitespace for consistent validation
  const trimmedAddress = address.trim();
  if (trimmedAddress.length > MAX_LENGTH_LIMIT) {
    console.debug(
      `validateOotleAddress: Invalid length ${trimmedAddress.length}, expected ${MAX_LENGTH_LIMIT} for address ${address}`,
    );
    return false;
  }

  try {
    decodeOotleAddress(trimmedAddress);
  } catch (e) {
    console.error("validateOotleAddress: decode error:", e, trimmedAddress);
    return false;
  }

  return true;
}
