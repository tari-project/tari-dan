import type { DryRunTransactionFinalizeResult } from "./DryRunTransactionFinalizeResult";
export interface SubmitTransactionResponse {
    transaction_id: string;
    dry_run_result: DryRunTransactionFinalizeResult | null;
}
