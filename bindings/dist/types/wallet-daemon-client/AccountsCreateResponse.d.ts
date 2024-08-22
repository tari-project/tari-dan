import type { FinalizeResult } from "../FinalizeResult";
import type { SubstateId } from "../SubstateId";
export interface AccountsCreateResponse {
    address: SubstateId;
    public_key: string;
    result: FinalizeResult;
}
