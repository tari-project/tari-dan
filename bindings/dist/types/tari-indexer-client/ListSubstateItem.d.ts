import type { SubstateId } from "../SubstateId";
export interface ListSubstateItem {
    substate_id: SubstateId;
    module_name: string | null;
    version: number;
    template_address: string | null;
    timestamp: bigint;
}
