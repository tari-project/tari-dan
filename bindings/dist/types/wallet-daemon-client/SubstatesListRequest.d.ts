import type { SubstateType } from "../SubstateType";
export interface SubstatesListRequest {
    filter_by_template: string | null;
    filter_by_type: SubstateType | null;
    limit: bigint | null;
    offset: bigint | null;
}
