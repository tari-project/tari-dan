import type { Epoch } from "./Epoch";
import type { Shard } from "./Shard";
export interface SubstateDestroyed {
    by_transaction: string;
    justify: string;
    by_block: string;
    at_epoch: Epoch;
    by_shard: Shard;
}
