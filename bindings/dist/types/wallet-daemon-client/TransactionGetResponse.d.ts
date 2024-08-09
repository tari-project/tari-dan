import type { FinalizeResult } from "../FinalizeResult";
import type { Transaction } from "../Transaction";
import type { TransactionStatus } from "../TransactionStatus";
export interface TransactionGetResponse {
    transaction: Transaction;
    result: FinalizeResult | null;
    status: TransactionStatus;
    last_update_time: string;
}
