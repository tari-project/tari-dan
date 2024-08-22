import type { Decision } from "./Decision";
import type { ExecuteResult } from "./ExecuteResult";
import type { RejectReason } from "./RejectReason";
import type { Transaction } from "./Transaction";
import type { VersionedSubstateIdLockIntent } from "./VersionedSubstateIdLockIntent";
export interface ExecutedTransaction {
    transaction: Transaction;
    result: ExecuteResult;
    resulting_outputs: Array<VersionedSubstateIdLockIntent>;
    resolved_inputs: Array<VersionedSubstateIdLockIntent>;
    final_decision: Decision | null;
    finalized_time: {
        secs: number;
        nanos: number;
    } | null;
    abort_reason: RejectReason | null;
}
