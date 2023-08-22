//   Copyright 2023 The Tari Project
//   SPDX-License-Identifier: BSD-3-Clause

use std::{
    fmt::{Debug, Formatter},
    hash::Hash,
    sync::{Arc, Mutex, MutexGuard},
};

#[derive(Clone)]
pub struct CMapSemaphore<K: Hash + Eq> {
    map: Arc<dashmap::DashMap<K, Arc<Mutex<()>>>>,
    global: Arc<std_semaphore::Semaphore>,
}

impl<K: Hash + Eq + Clone> CMapSemaphore<K> {
    pub fn new(max_global_access: isize) -> Self {
        Self {
            map: Arc::new(dashmap::DashMap::new()),
            global: Arc::new(std_semaphore::Semaphore::new(max_global_access)),
        }
    }

    pub fn acquire(&self, key: K) -> CMapSemaphoreGuard<'_, K> {
        let global_access = self.global.access();
        let map_mutex = self
            .map
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone();
        CMapSemaphoreGuard {
            _global_access: global_access,
            map: self.map.clone(),
            map_mutex,
            key,
        }
    }
}

impl<K: Hash + Eq> Debug for CMapSemaphore<K> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CMapSemaphore")
            .field("map", &self.map.len())
            .field("global", &"...")
            .finish()
    }
}

pub struct CMapSemaphoreGuard<'a, K: Hash + Eq> {
    _global_access: std_semaphore::SemaphoreGuard<'a>,
    map: Arc<dashmap::DashMap<K, Arc<Mutex<()>>>>,
    map_mutex: Arc<Mutex<()>>,
    key: K,
}

impl<'a, K: Hash + Eq> CMapSemaphoreGuard<'a, K> {
    pub fn access(&self) -> MutexGuard<'_, ()> {
        // Unwrap: only errors if the mutex is poisoned, which is a bug
        self.map_mutex.lock().unwrap()
    }
}

impl<K: Hash + Eq> Drop for CMapSemaphoreGuard<'_, K> {
    fn drop(&mut self) {
        self.map.remove(&self.key);
    }
}
