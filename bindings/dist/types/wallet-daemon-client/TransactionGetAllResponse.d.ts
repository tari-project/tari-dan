import type { FinalizeResult } from "../FinalizeResult";
import type { Transaction } from "../Transaction";
import type { TransactionStatus } from "../TransactionStatus";
export interface TransactionGetAllResponse {
    transactions: Array<[Transaction, FinalizeResult | null, TransactionStatus, string]>;
}
