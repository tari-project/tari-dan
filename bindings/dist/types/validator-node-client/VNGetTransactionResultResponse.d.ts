import type { Decision } from "../Decision";
import type { ExecuteResult } from "../ExecuteResult";
export interface VNGetTransactionResultResponse {
    result: ExecuteResult | null;
    final_decision: Decision | null;
    finalized_time: {
        secs: number;
        nanos: number;
    } | null;
    execution_time: {
        secs: number;
        nanos: number;
    } | null;
}
