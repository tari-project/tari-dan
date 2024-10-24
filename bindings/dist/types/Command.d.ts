import type { ForeignProposalAtom } from "./ForeignProposalAtom";
import type { MintConfidentialOutputAtom } from "./MintConfidentialOutputAtom";
import type { ResumeNodeAtom } from "./ResumeNodeAtom";
import type { SuspendNodeAtom } from "./SuspendNodeAtom";
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
    ForeignProposal: ForeignProposalAtom;
} | {
    MintConfidentialOutput: MintConfidentialOutputAtom;
} | {
    SuspendNode: SuspendNodeAtom;
} | {
    ResumeNode: ResumeNodeAtom;
} | "EndEpoch";
