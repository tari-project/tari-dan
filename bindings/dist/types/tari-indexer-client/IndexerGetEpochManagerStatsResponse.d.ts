import type { Epoch } from "../Epoch";
export interface IndexerGetEpochManagerStatsResponse {
    current_epoch: Epoch;
    current_block_height: number;
    current_block_hash: string;
}
