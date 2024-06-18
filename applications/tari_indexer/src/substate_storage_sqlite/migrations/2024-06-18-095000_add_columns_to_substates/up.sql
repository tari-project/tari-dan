-- The transaction hash column will be similar to the one in the "events" table
alter table substates
    drop column transaction_hash;
alter table substates
    add column tx_hash text not NULL;

alter table substates
    add column template_address text NULL;

alter table substates
    add column module_name text NULL;