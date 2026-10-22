//   Copyright 2025 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause
import CopyToClipboard from "@components/CopyToClipboard";
import StatusChip, { StatusChipColors } from "@components/StatusChip";
import { Memo as TyMemo } from "@tari-project/ootle-ts-bindings";
import { hexToU8 } from "cbor2/utils";

export type MemoProps = {
  memo?: TyMemo | null;
};

export function Memo({ memo }: MemoProps) {
  if (!memo) {
    return <span>--</span>;
  }

  if ("Message" in memo) {
    return <span>{memo ? memo.Message : "No Memo"}</span>;
  }
  if ("Bytes" in memo) {
    return <span>{memo ? Buffer.from(memo.Bytes).toString("hex") : "No Memo"}</span>;
  }
  if ("SenderAddress" in memo) {
    // The resolved Ootle address is surfaced separately (see StealthUtxoList); show a label if we end up here.
    return <span>Sender address</span>;
  }
  if ("PayRefAndBytes" in memo) {
    try {
      const bytes = hexToU8(memo.PayRefAndBytes);
      const len = bytes[0];
      const payRef = tryDecodeUtf8(bytes.slice(1, 1 + len)) || Buffer.from(bytes.slice(1, 1 + len)).toString("hex");
      const message = bytes.length >= 1 + len ? tryDecodeUtf8(bytes.slice(1 + len)) : [];
      return (
        <span>
          {message}
          <span style={{ marginLeft: "8px" }}>
            {payRef && (
              <StatusChip color={StatusChipColors.Blue} title="This UTXO has an attached Payment Ref.">
                <CopyToClipboard copy={payRef} /> {payRef}{" "}
              </StatusChip>
            )}
          </span>
        </span>
      );
    } catch (e) {
      console.warn("Failed to decode PayRefAndBytes memo:", e);
      // ignore
    }
  }

  return <span>{JSON.stringify(memo)}</span>;
}

function tryDecodeUtf8(bytes: Uint8Array): string | null {
  try {
    let decoder = new TextDecoder();
    return decoder.decode(bytes);
  } catch (e) {
    return null;
  }
}
