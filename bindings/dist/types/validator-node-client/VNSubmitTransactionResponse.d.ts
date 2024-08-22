import type { DryRunTransactionFinalizeResult } from "./DryRunTransactionFinalizeResult";
export interface VNSubmitTransactionResponse {
    transaction_id: string;
    dry_run_result: DryRunTransactionFinalizeResult | null;
}
