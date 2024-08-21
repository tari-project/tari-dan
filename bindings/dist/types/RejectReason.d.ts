export type RejectReason = {
    InvalidTransaction: string;
} | {
    ExecutionFailure: string;
} | {
    OneOrMoreInputsNotFound: string;
} | "NoInputs" | {
    FailedToLockInputs: string;
} | {
    FailedToLockOutputs: string;
} | {
    ForeignShardGroupDecidedToAbort: string;
} | {
    FeesNotPaid: string;
} | "Unknown";
