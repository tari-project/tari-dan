// This file was generated by [ts-rs](https://github.com/Aleph-Alpha/ts-rs). Do not edit this file manually.
import type { FinalizeResult } from "../FinalizeResult";
import type { Transaction } from "../Transaction";
import type { TransactionStatus } from "../TransactionStatus";

export interface TransactionGetResponse { transaction: Transaction, result: FinalizeResult | null, status: TransactionStatus, last_update_time: string, }