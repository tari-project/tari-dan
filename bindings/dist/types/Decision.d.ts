import type { AbortReason } from "./AbortReason";
export type Decision = "Commit" | {
    Abort: AbortReason;
};
