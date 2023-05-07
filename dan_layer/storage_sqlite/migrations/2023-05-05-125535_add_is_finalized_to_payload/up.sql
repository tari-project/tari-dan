ALTER TABLE main.payloads
    ADD COLUMN is_finalized       boolean NOT NULL DEFAULT '0';