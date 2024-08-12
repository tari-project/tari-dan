import type { Transaction } from "../Transaction";
export interface SubmitTransactionRequest {
    transaction: Transaction;
    is_dry_run: boolean;
}
