// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { SubstateId } from "../SubstateId";

export interface GetSubstateRequest {
  address: SubstateId;
  version: number | null;
  local_search_only: boolean;
}
