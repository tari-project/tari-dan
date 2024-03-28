ALTER TABLE transactions
    ADD COLUMN required_substates text NOT NULL default '[]';
ALTER TABLE transactions
    ADD COLUMN new_account_info text NULL;
ALTER TABLE transactions
    DROP COLUMN json_result;
