import type { ForeignProposal } from "./ForeignProposal";
import type { TransactionAtom } from "./TransactionAtom";
export type Command = {
    Prepare: TransactionAtom;
} | {
    LocalPrepared: TransactionAtom;
} | {
    Accept: TransactionAtom;
} | {
    ForeignProposal: ForeignProposal;
} | {
    LocalOnly: TransactionAtom;
} | "EndEpoch";
