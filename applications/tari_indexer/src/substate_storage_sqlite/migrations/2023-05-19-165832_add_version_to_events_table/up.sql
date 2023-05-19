alter table events
    add column version integer not null;
    
alter table table_name 
    rename column template_address TO component_address;