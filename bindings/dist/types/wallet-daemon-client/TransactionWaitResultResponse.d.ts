import type { Amount } from "../Amount";
import type { FinalizeResult } from "../FinalizeResult";
import type { TransactionStatus } from "../TransactionStatus";
export interface TransactionWaitResultResponse {
    transaction_id: string;
    result: FinalizeResult | null;
    json_result: Array<any> | null;
    status: TransactionStatus;
    final_fee: Amount;
    timed_out: boolean;
}
