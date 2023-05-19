alter table events
    add column version integer not null;
    
alter table events 
    rename column template_address TO component_address;