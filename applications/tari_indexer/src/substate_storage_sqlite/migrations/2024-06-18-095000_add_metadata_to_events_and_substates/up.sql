-- The transaction hash column will be similar to the one in the "events" table
alter table substates
    drop column transaction_hash;
alter table substates
    add column tx_hash text not NULL;

-- Tempalte address for component substates
alter table substates
    add column template_address text NULL;

-- Name of the template module for components
alter table substates
    add column module_name text NULL;

-- block timestamp for events and substates
alter table events
    add column timestamp bigint not NULL;
alter table substates
    add column timestamp bigint not NULL;