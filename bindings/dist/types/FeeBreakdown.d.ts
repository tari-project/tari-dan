import type { FeeSource } from "./FeeSource";
export interface FeeBreakdown {
    breakdown: Record<FeeSource, bigint>;
}
