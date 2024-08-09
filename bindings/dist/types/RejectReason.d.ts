export type RejectReason = {
    ShardsNotPledged: string;
} | {
    ExecutionFailure: string;
} | "PreviousQcRejection" | {
    ShardPledgedToAnotherPayload: string;
} | {
    ShardRejected: string;
} | "FeeTransactionFailed" | {
    FeesNotPaid: string;
};
