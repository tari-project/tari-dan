import type { Amount } from "./Amount";
import type { FeeBreakdown } from "./FeeBreakdown";
export interface FeeReceipt {
    total_fee_payment: Amount;
    total_fees_paid: Amount;
    cost_breakdown: Array<FeeBreakdown>;
}
