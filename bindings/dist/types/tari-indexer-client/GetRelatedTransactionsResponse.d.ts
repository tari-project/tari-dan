import type { IndexerTransactionFinalizedResult } from "./IndexerTransactionFinalizedResult";
export interface GetRelatedTransactionsResponse {
    transaction_results: Array<IndexerTransactionFinalizedResult>;
}
