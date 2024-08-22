import type { FeeCostBreakdown } from "../FeeCostBreakdown";
import type { FinalizeResult } from "../FinalizeResult";
import type { QuorumDecision } from "../QuorumDecision";
export interface DryRunTransactionFinalizeResult {
    decision: QuorumDecision;
    finalize: FinalizeResult;
    fee_breakdown: FeeCostBreakdown | null;
}
