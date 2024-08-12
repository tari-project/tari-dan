import type { IndexerTransactionFinalizedResult } from "./IndexerTransactionFinalizedResult";
export interface SubmitTransactionResponse {
    transaction_id: string;
    result: IndexerTransactionFinalizedResult;
}
