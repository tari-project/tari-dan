import type { Shard } from "./Shard";
export interface ShardGroup {
    start: Shard;
    end_inclusive: Shard;
}
