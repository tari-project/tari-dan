ALTER TABLE substates ADD COLUMN version_narrow INTEGER NOT NULL DEFAULT 0;
UPDATE substates SET version_narrow = version;
ALTER TABLE substates DROP COLUMN version;
ALTER TABLE substates RENAME COLUMN version_narrow TO version;

ALTER TABLE vaults ADD COLUMN vault_version_narrow INTEGER NOT NULL DEFAULT 0;
UPDATE vaults SET vault_version_narrow = vault_version;
ALTER TABLE vaults DROP COLUMN vault_version;
ALTER TABLE vaults RENAME COLUMN vault_version_narrow TO vault_version;
