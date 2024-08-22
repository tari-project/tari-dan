import type { Epoch } from "../Epoch";
import type { SubstateAddress } from "../SubstateAddress";
export interface GetCommitteeRequest {
    epoch: Epoch;
    substate_address: SubstateAddress;
}
