import type { Transaction } from "../Transaction";
export interface VNSubmitTransactionRequest {
    transaction: Transaction;
    is_dry_run: boolean;
}
