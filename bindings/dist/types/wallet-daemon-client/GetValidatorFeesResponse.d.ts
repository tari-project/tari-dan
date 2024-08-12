import type { Amount } from "../Amount";
import type { Epoch } from "../Epoch";
export interface GetValidatorFeesResponse {
    fee_summary: Record<Epoch, Amount>;
}
