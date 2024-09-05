import type { RejectReason } from "./RejectReason";
import type { SubstateDiff } from "./SubstateDiff";
export type TransactionResult = {
    Accept: SubstateDiff;
} | {
    AcceptFeeRejectRest: [SubstateDiff, RejectReason];
} | {
    Reject: RejectReason;
};
