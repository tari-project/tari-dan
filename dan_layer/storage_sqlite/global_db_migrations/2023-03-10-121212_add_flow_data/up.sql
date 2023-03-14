--  // Copyright 2022 The Tari Project
--  // SPDX-License-Identifier: BSD-3-Clause


drop table templates;

create table templates
(
    id               Integer primary key autoincrement not null,
    -- template name
    template_name    text                              not null,
    -- the address is the hash of the content
    template_address blob                              not null,
    -- where to find the template code
    url              text                              not null,
    -- the block height in which the template was published
    height           bigint                            not null,
    -- The type of template, used to create an enum in code
    template_type    text                              not null,

    -- compiled template code as a WASM binary
    compiled_code    blob                              null,
    -- flow json
    flow_json        text                              null,
    status VARCHAR(20) NOT NULL DEFAULT 'New',
    wasm_path VARCHAR(255) NULL,
    manifest text null,
    added_at timestamp NOT NULL DEFAULT CURRENT_TIMESTAMP
);

-- fetching by the template_address will be a very common operation
create unique index templates_template_address_index on templates (template_address);
