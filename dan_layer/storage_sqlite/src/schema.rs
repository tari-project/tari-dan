//  Copyright 2022. The Tari Project
//
//  Redistribution and use in source and binary forms, with or without modification, are permitted provided that the
//  following conditions are met:
//
//  1. Redistributions of source code must retain the above copyright notice, this list of conditions and the following
//  disclaimer.
//
//  2. Redistributions in binary form must reproduce the above copyright notice, this list of conditions and the
//  following disclaimer in the documentation and/or other materials provided with the distribution.
//
//  3. Neither the name of the copyright holder nor the names of its contributors may be used to endorse or promote
//  products derived from this software without specific prior written permission.
//
//  THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS" AND ANY EXPRESS OR IMPLIED WARRANTIES,
//  INCLUDING, BUT NOT LIMITED TO, THE IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE
//  DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL,
//  SPECIAL, EXEMPLARY, OR CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR
//  SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY,
//  WHETHER IN CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE
//  USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.

table! {
    instructions (id) {
        id -> Integer,
        hash -> Binary,
        node_id -> Integer,
        template_id -> Integer,
        method -> Text,
        args -> Binary,
        sender -> Binary,
    }
}

table! {
    locked_qc (id) {
        id -> Integer,
        message_type -> Integer,
        view_number -> BigInt,
        node_hash -> Binary,
        signature -> Nullable<Binary>,
    }
}

table! {
    prepare_qc (id) {
        id -> Integer,
        message_type -> Integer,
        view_number -> BigInt,
        node_hash -> Binary,
        signature -> Nullable<Binary>,
    }
}

table! {
    state_keys (schema_name, key_name) {
        schema_name -> Text,
        key_name -> Binary,
        value -> Binary,
    }
}

table! {
    state_op_log (id) {
        id -> Integer,
        height -> BigInt,
        merkle_root -> Nullable<Binary>,
        operation -> Text,
        schema -> Text,
        key -> Binary,
        value -> Nullable<Binary>,
    }
}

table! {
    state_tree (id) {
        id -> Integer,
        version -> Integer,
        is_current -> Bool,
        data -> Binary,
    }
}


table! {
    payloads (id) {
        id -> Integer,
        payload_id -> Binary,
        instructions -> Binary,
        public_nonce -> Binary,
        scalar -> Binary,
        fee -> BigInt,
        sender_public_key -> Binary,
        meta -> Binary,
    }
}

table! {
    votes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        shard_id -> Binary,
        address -> Binary,
        node_height -> BigInt,
        vote_message -> Binary,
    }
}

table! {
    leaf_nodes (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

table! {
    last_voted_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

table! {
    lock_node_and_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        tree_node_hash -> Binary,
        node_height -> BigInt,
    }
}

table! {
    nodes (id) {
        id -> Integer,
        tree_node_hash -> Binary,
        parent_node_hash -> Binary,
        height -> BigInt,
        shard -> Binary,
        payload_id -> Binary,
        payload_height -> BigInt,
        local_pledges -> Binary,
        epoch -> BigInt,
        proposed_by -> Binary,
        justify -> Binary,
    }
}

table! {
    last_executed_heights (id) {
        id -> Integer,
        shard_id -> Binary,
        node_height -> BigInt,
    }
}

table! {
    payload_votes (id) {
        id -> Integer,
        payload_id -> Binary,
        shard_id -> Binary,
        node_height -> BigInt,
        hotstuff_tree_node -> Binary,
    }
}

table! {
    objects (id) {
        id -> Integer,
        shard_id -> Binary,
        object_id -> Binary,
        payload_id -> Binary,
        node_height -> BigInt,
        substate_change -> Binary,
        object_pledge -> Binary,
    }
}

table! {
    substate_changes (id) {
        id -> Integer,
        shard_id -> Binary,
        substate_change -> Binary,
        tree_node_hash -> Binary,
    }
}

table! {
    high_qcs (id) {
        id -> Integer,
        shard_id -> Binary,
        height -> BigInt,
        is_highest -> Integer,
        qc_json -> Text,
    }
}

table! {
    metadata (key) {
        key -> Binary,
        value -> Binary,
    }
}

table! {
    templates (id) {
        id -> Integer,
        template_address -> Binary,
        url -> Text,
        height -> Integer,
        compiled_code -> Binary,
    }
}

joinable!(instructions -> nodes (node_id));

allow_tables_to_appear_in_same_query!(
    instructions,
    locked_qc,
    metadata,
    prepare_qc,
    state_keys,
    state_op_log,
    state_tree,
    high_qcs,
    payloads,
    votes,
    leaf_nodes,
    last_voted_heights,
    lock_node_and_heights,
    nodes,
    last_executed_heights,
    payload_votes,
    objects,
    substate_changes,
    templates,
);
