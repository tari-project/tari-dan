import type { SubstateStatus } from "./SubstateStatus";
import type { SubstateValue } from "../SubstateValue";
export interface GetSubstateResponse {
    value: SubstateValue | null;
    created_by_tx: string | null;
    status: SubstateStatus;
}
