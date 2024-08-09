import type { Decision } from "./Decision";
import type { ExecuteResult } from "./ExecuteResult";
import type { Transaction } from "./Transaction";
import type { VersionedSubstateId } from "./VersionedSubstateId";
import type { VersionedSubstateIdLockIntent } from "./VersionedSubstateIdLockIntent";
export interface ExecutedTransaction {
    transaction: Transaction;
    result: ExecuteResult;
    resulting_outputs: Array<VersionedSubstateId>;
    resolved_inputs: Array<VersionedSubstateIdLockIntent>;
    execution_time: {
        secs: number;
        nanos: number;
    };
    final_decision: Decision | null;
    finalized_time: {
        secs: number;
        nanos: number;
    } | null;
    abort_details: string | null;
}
