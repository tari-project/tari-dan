import type { Decision } from "../Decision";
import type { ExecuteResult } from "../ExecuteResult";
export type IndexerTransactionFinalizedResult = "Pending" | {
    "Finalized": {
        final_decision: Decision;
        execution_result: ExecuteResult | null;
        execution_time: {
            secs: number;
            nanos: number;
        };
        finalized_time: {
            secs: number;
            nanos: number;
        };
        abort_details: string | null;
        json_results: Array<string>;
    };
};
