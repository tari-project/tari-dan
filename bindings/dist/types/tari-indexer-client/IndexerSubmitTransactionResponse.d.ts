import type { IndexerTransactionFinalizedResult } from "./IndexerTransactionFinalizedResult";
export interface IndexerSubmitTransactionResponse {
    transaction_id: string;
    result: IndexerTransactionFinalizedResult;
}
