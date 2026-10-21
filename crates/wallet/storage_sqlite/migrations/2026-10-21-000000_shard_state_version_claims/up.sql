-- Each shard's stealth scan cursor as an indexer reported it, past the cursor stored in `shard_state_versions`, which
-- is bounded by a verified checkpoint. A scan resumes from the claim until a checkpoint that ended after it shows the
-- network never reached it.
--
-- `verified_epoch` is the epoch of the latest verified checkpoint of the shard when the claim was stored, NULL without
-- one.
CREATE TABLE shard_state_version_claims
(
    id             INTEGER  NOT NULL PRIMARY KEY AUTOINCREMENT,
    account_id     INTEGER  NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    resource_id    INTEGER  NOT NULL REFERENCES resources (id) ON DELETE CASCADE,
    shard          INTEGER  NOT NULL,
    state_version  BIGINT   NOT NULL,
    verified_epoch BIGINT   NULL,
    created_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at     DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE UNIQUE INDEX shard_state_version_claims_account_resource_shard_uniq ON shard_state_version_claims (account_id, resource_id, shard);
