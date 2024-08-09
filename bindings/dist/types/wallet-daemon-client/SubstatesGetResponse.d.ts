import type { Substate } from "../Substate";
import type { WalletSubstateRecord } from "./WalletSubstateRecord";
export interface SubstatesGetResponse {
    record: WalletSubstateRecord;
    value: Substate;
}
