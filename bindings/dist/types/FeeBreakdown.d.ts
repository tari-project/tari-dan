import type { FeeSource } from "./FeeSource";
export interface FeeBreakdown {
    source: FeeSource;
    amount: number;
}
