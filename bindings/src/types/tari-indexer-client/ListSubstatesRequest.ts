// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { SubstateType } from "./SubstateType";

export interface ListSubstatesRequest {
  filter_by_template: string | null;
  filter_by_type: SubstateType | null;
  limit: bigint | null;
  offset: bigint | null;
}