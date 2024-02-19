// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { ExecuteResult } from "../ExecuteResult";
import type { SubstateRequirement } from "../SubstateRequirement";

export interface TransactionSubmitResponse {
  transaction_id: string;
  inputs: Array<SubstateRequirement>;
  result: ExecuteResult | null;
  json_result: Array<any> | null;
}