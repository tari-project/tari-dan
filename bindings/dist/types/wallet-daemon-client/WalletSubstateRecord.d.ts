import type { SubstateId } from "../SubstateId";
export interface WalletSubstateRecord {
    substate_id: SubstateId;
    parent_id: SubstateId | null;
    module_name: string | null;
    version: number;
    template_address: string | null;
}
