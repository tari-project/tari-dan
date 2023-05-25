alter table events
    add column version integer not null;
    
alter table events 
    add column component_address string null;

-- drop previous index 
drop index unique_events_indexer;

-- add new `unique_events_indexer`
create unique index unique_events_indexer on events (template_address, tx_hash, topic, component_address, version);
