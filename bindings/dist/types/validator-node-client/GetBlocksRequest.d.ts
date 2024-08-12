import type { Ordering } from "../Ordering";
export interface GetBlocksRequest {
    limit: number;
    offset: number;
    ordering_index: number | null;
    ordering: Ordering | null;
    filter_index: number | null;
    filter: string | null;
}
