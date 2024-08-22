import type { Transaction } from "../Transaction";
export interface GetRecentTransactionsResponse {
    transactions: Array<Transaction>;
}
