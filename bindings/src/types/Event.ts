// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { Metadata } from "./Metadata";

export interface Event {
  substate_id: string | null;
  template_address: string;
  tx_hash: string;
  topic: string;
  payload: Metadata;
}
