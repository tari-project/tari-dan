import type { FinalizeResult } from "../FinalizeResult";
import type { TransactionStatus } from "../TransactionStatus";
export interface TransactionGetResultResponse {
    transaction_id: string;
    status: TransactionStatus;
    result: FinalizeResult | null;
    json_result: Array<any> | null;
}
