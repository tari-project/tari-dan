-- The caller a lock was created for: NULL for the wallet itself, 'delegated'
-- for a connected app, or 'api_key:<id>' for an API key. Only that caller may
-- name the lock in a transaction request.
ALTER TABLE locks
    ADD COLUMN owner TEXT NULL;
