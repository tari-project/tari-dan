--  // Copyright 2021. The Tari Project
--  //
--  // Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
--  // following conditions are met:
--  //
--  // 1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
--  // disclaimer.
--  //
--  // 2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
--  // following disclaimer in the documentation and/or other materials provided with the distribution.
--  //
--  // 3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
--  // products derived from this software without specific prior written permission.
--  //
--  // THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
--  // INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
--  // DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
--  // SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
--  // SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
--  // WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
--  // USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

create table templates
(
    id               Integer primary key autoincrement not null,
    -- the address is the hash of the content
    template_address blob                              not null,
    -- where to find the template code
    url              text                              not null,
    -- the block height in which the template was published
    height           bigint                            not null,
    -- compiled template code as a WASM binary
    compiled_code    blob                              not null
);

-- fetching by the template_address will be a very common operation
create unique index templates_template_address_index on templates (template_address);
