//   Copyright 2022 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use tari_template_lib::Hash;

use crate::{
    runtime::{IdProvider, RuntimeState, StateTracker},
    state_store::memory::MemoryStateStore,
};

mod tracker {

    use super::*;

    #[test]
    fn it_creates_a_new_component() {
        let store = MemoryStateStore::default();
        let tx_hash = Hash::default();
        let id_provider = IdProvider::new(tx_hash, 1);
        let tracker = StateTracker::new(store, id_provider);
        tracker.set_current_runtime_state(RuntimeState {
            template_address: Default::default(),
        });
        let addr = tracker.new_component("test".to_string(), vec![1, 2, 3]).unwrap();
        let component = tracker.get_component(&addr).unwrap();
        assert_eq!(component.module_name, "test");
        assert_eq!(component.state.state, vec![1, 2, 3]);
    }
}
