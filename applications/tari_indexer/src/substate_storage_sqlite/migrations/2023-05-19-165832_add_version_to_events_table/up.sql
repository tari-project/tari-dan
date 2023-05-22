alter table events
    add column version integer not null;
    
alter table events 
    add column component_address string null;