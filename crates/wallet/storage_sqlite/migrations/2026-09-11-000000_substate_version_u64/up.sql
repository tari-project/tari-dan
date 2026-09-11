-- Widen the wallet's substate version columns to hold a u64.
--
-- A version counts one write per transaction that touches the substate, so its ceiling is a
-- throughput ceiling on a single contended substate, not a calendar one. The column must hold the
-- full range the protocol admits, which a signed 32-bit integer does not.
--
-- SQLite cannot change a column's declared type in place, and a table rebuild is not available here:
-- `vaults` is a foreign key target, and inside the transaction a migration runs in, renaming it
-- repoints every referencing table at the temporary name. Adding the wider column and dropping the
-- narrow one leaves those references untouched. Both columns move to the end of their table as a
-- result, which is where the generated schema expects them.

ALTER TABLE substates ADD COLUMN version_wide BIGINT NOT NULL DEFAULT 0;
UPDATE substates SET version_wide = version;
ALTER TABLE substates DROP COLUMN version;
ALTER TABLE substates RENAME COLUMN version_wide TO version;

ALTER TABLE vaults ADD COLUMN vault_version_wide BIGINT NOT NULL DEFAULT 0;
UPDATE vaults SET vault_version_wide = vault_version;
ALTER TABLE vaults DROP COLUMN vault_version;
ALTER TABLE vaults RENAME COLUMN vault_version_wide TO vault_version;
