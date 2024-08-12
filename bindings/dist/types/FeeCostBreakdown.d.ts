import type { Amount } from "./Amount";
import type { FeeBreakdown } from "./FeeBreakdown";
export interface FeeCostBreakdown {
    total_fees_charged: Amount;
    breakdown: Array<FeeBreakdown>;
}
