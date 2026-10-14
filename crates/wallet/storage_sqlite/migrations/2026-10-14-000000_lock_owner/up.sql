-- The caller a lock was created for: NULL for the wallet itself,
-- 'delegated:<id>' for a connected app's session, or 'api_key:<id>' for an
-- API key. Only that caller may name the lock in a transaction request or
-- cancel it.
ALTER TABLE locks
    ADD COLUMN owner TEXT NULL;
