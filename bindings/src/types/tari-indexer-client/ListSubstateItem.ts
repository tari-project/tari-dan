// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { SubstateId } from "../SubstateId";

export interface ListSubstateItem {
  substate_id: SubstateId;
  module_name: string | null;
  version: number;
  template_address: string | null;
  timestamp: bigint;
}
