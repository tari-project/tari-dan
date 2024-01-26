ALTER TABLE transactions
    ADD COLUMN executed_time_ms bigint NULL;

ALTER TABLE transactions
    ADD COLUMN finalized_time_ms bigint NULL;
