-- The validator sets the wallet trusts, one per epoch, each a JSON `TrustedValidatorSetHash`.
--
-- Invariants:
--   * Every row descends from one anchor: the row whose `anchor` is non-NULL
--     ('FirstUse' or 'Reanchor'). Trust in every later row comes from a handover by the
--     set of the epoch before it. Anchoring anew deletes every row first.
--   * A row is never changed once written: a set verified for an epoch stays verified.
CREATE TABLE trusted_validator_sets (
    epoch         BIGINT   NOT NULL PRIMARY KEY,
    validator_set TEXT     NOT NULL,
    anchor        TEXT     NULL,
    created_at    DATETIME NOT NULL DEFAULT CURRENT_TIMESTAMP
);
