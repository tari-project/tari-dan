import type { Event } from "./Event";
import type { FeeReceipt } from "./FeeReceipt";
import type { LogEntry } from "./LogEntry";
export interface TransactionReceipt {
    transaction_hash: Uint8Array;
    events: Array<Event>;
    logs: Array<LogEntry>;
    fee_receipt: FeeReceipt;
}
