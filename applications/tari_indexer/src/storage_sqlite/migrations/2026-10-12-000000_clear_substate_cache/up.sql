-- Cached component states written before `Value::Null` encoded as CBOR null hold an empty array in
-- place of every null. Dropping the cache makes every substate refetch from the committee.
delete from substate_cache_proofs;
delete from substate_cache;
