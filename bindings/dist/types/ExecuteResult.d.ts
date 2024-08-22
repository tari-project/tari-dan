import type { FinalizeResult } from "./FinalizeResult";
export interface ExecuteResult {
    finalize: FinalizeResult;
    execution_time: {
        secs: number;
        nanos: number;
    };
}
