import type { SubstateId } from "./SubstateId";
export interface Account {
    name: string | null;
    address: SubstateId;
    key_index: number;
    is_default: boolean;
}
