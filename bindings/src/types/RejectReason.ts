// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.

export type RejectReason =
  | { InvalidTransaction: string }
  | { ExecutionFailure: string }
  | { OneOrMoreInputsNotFound: string }
  | "NoInputs"
  | { FailedToLockInputs: string }
  | { FailedToLockOutputs: string }
  | { ForeignShardGroupDecidedToAbort: string }
  | { FeesNotPaid: string }
  | "Unknown";
