import type { Metadata } from "./Metadata";
import type { SubstateId } from "./SubstateId";
export interface Event {
    substate_id: SubstateId | null;
    template_address: string;
    tx_hash: string;
    topic: string;
    payload: Metadata;
}
