// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { SubstateStatus } from "./SubstateStatus";
import type { SubstateValue } from "./SubstateValue";

export interface GetSubstateResponse {
  value: SubstateValue | null;
  created_by_tx: string | null;
  status: SubstateStatus;
}
