// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { DryRunTransactionFinalizeResult } from "./DryRunTransactionFinalizeResult";

export interface SubmitTransactionResponse {
  transaction_id: string;
  dry_run_result: DryRunTransactionFinalizeResult | null;
}
