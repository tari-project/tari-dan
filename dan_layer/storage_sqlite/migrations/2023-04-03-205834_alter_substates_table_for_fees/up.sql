ALTER TABLE substates
    ADD COLUMN fee_paid_for_created_justify bigint not NULL;

ALTER TABLE substates
    ADD COLUMN fee_paid_for_deleted_justify bigint not NULL;

ALTER TABLE substates
    ADD COLUMN created_at_epoch             bigint NULL;

ALTER TABLE substates
    ADD COLUMN destroyed_at_epoch             bigint NULL;

ALTER TABLE substates
    ADD COLUMN created_justify_leader       text NULL;

ALTER TABLE substates
    ADD COLUMN destroyed_justify_leader       text NULL;