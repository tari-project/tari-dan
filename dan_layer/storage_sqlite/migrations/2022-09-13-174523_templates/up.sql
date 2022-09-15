create table templates (
    id                  Integer primary key autoincrement not null,
    -- the address is the hash of the content
    template_address    blob not null,
    -- where to find the template code
    url                 text not null,                              
    -- the block height in which the template was published
    height              bigint not null,
    -- compiled template code as a WASM binary 
    compiled_code       blob not null
);

-- fetching by the template_address will be a very common operation
create index templates_template_address_index on templates (template_address);