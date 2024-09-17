export type RejectReason = {
    InvalidTransaction: string;
} | {
    ExecutionFailure: string;
} | {
    OneOrMoreInputsNotFound: string;
} | {
    FailedToLockInputs: string;
} | {
    FailedToLockOutputs: string;
} | {
    ForeignShardGroupDecidedToAbort: {
        start_shard: number;
        end_shard: number;
    };
} | {
    FeesNotPaid: string;
} | "Unknown";
