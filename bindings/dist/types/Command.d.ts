import type { ForeignProposal } from "./ForeignProposal";
import type { TransactionAtom } from "./TransactionAtom";
export type Command = {
    LocalOnly: TransactionAtom;
} | {
    Prepare: TransactionAtom;
} | {
    LocalPrepare: TransactionAtom;
} | {
    AllPrepare: TransactionAtom;
} | {
    SomePrepare: TransactionAtom;
} | {
    LocalAccept: TransactionAtom;
} | {
    AllAccept: TransactionAtom;
} | {
    SomeAccept: TransactionAtom;
} | {
    ForeignProposal: ForeignProposal;
} | "EndEpoch";
