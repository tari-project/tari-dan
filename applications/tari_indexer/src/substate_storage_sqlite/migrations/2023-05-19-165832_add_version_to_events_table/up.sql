alter table events
    add column version integer not null;
    
alter table events 
    add column component_address text null;

-- drop previous index 
drop index unique_events_indexer;
