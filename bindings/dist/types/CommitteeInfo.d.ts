import type { NumPreshards } from "./NumPreshards";
import type { ShardGroup } from "./ShardGroup";
export interface CommitteeInfo {
    num_shards: NumPreshards;
    num_shard_group_members: number;
    num_committees: number;
    shard_group: ShardGroup;
}
