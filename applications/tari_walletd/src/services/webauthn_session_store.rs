// Copyright 2025 The Tari Project
// SPDX-License-Identifier: BSD-3-Clause

use std::{
    collections::{BTreeSet, HashMap},
    sync::Arc,
    time::{Duration, Instant},
};

use tari_ootle_common_types::optional::IsNotFoundError;
use thiserror::Error;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Clone, Debug, Error)]
pub enum SessionStoreError {
    #[error("Session not found: {session_id}")]
    SessionNotFound { session_id: String },
}

impl IsNotFoundError for SessionStoreError {
    fn is_not_found_error(&self) -> bool {
        matches!(self, SessionStoreError::SessionNotFound { .. })
    }
}

/// A trait the every session data must implement.
pub trait SessionData {
    fn created_at(&self) -> Instant;
}

/// A thread-safe store that acts like a classical Session storage for web, but uses unique IDs for a session
/// instead of using cookies, so it is suitable for stateless RPC calls.
///
/// The store holds at most `max_sessions` sessions. Adding a session to a full store evicts the oldest one, so
/// sessions may be created by unauthenticated callers without the store growing past its bound.
#[derive(Debug, Clone)]
pub struct SessionStore<T> {
    sessions: Arc<RwLock<Sessions<T>>>,
    session_ttl: Duration,
    max_sessions: usize,
}

impl<T: SessionData + Clone> SessionStore<T> {
    pub fn new(session_ttl: Duration, max_sessions: usize) -> Self {
        Self {
            sessions: Arc::new(RwLock::new(Sessions::new())),
            session_ttl,
            max_sessions,
        }
    }

    /// Get session by ID.
    pub async fn get(&self, session_id: &str) -> Result<T, SessionStoreError> {
        let sessions = self.sessions.read().await;
        sessions
            .by_id
            .get(session_id)
            .filter(|session| !self.is_expired(*session))
            .cloned()
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    /// Add new session
    pub async fn add(&self, data: T) -> Result<String, SessionStoreError> {
        let mut sessions = self.sessions.write().await;
        sessions.remove_expired(self.session_ttl);
        while sessions.by_id.len() >= self.max_sessions.max(1) {
            sessions.remove_oldest();
        }
        let mut session_id = Uuid::new_v4().to_string();
        while sessions.by_id.contains_key(&session_id) {
            session_id = Uuid::new_v4().to_string();
        }
        sessions.insert(session_id.clone(), data);
        Ok(session_id)
    }

    /// Removes session, if it does not exists, this call have no effect.
    pub async fn remove(&self, session_id: &str) -> Result<T, SessionStoreError> {
        let mut sessions = self.sessions.write().await;
        sessions
            .remove(session_id)
            .ok_or_else(|| SessionStoreError::SessionNotFound {
                session_id: session_id.to_string(),
            })
    }

    fn is_expired(&self, session: &T) -> bool {
        session.created_at().elapsed() > self.session_ttl
    }
}

/// Sessions indexed by ID and by creation time, so that expiry and eviction only touch the sessions they remove.
#[derive(Debug)]
struct Sessions<T> {
    by_id: HashMap<String, T>,
    by_age: BTreeSet<(Instant, String)>,
}

impl<T: SessionData> Sessions<T> {
    fn new() -> Self {
        Self {
            by_id: HashMap::new(),
            by_age: BTreeSet::new(),
        }
    }

    fn insert(&mut self, session_id: String, data: T) {
        self.by_age.insert((data.created_at(), session_id.clone()));
        self.by_id.insert(session_id, data);
    }

    fn remove(&mut self, session_id: &str) -> Option<T> {
        let session = self.by_id.remove(session_id)?;
        self.by_age.remove(&(session.created_at(), session_id.to_string()));
        Some(session)
    }

    fn remove_oldest(&mut self) {
        if let Some((_, session_id)) = self.by_age.pop_first() {
            self.by_id.remove(&session_id);
        }
    }

    fn remove_expired(&mut self, session_ttl: Duration) {
        while self
            .by_age
            .first()
            .is_some_and(|(created_at, _)| created_at.elapsed() > session_ttl)
        {
            self.remove_oldest();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone)]
    struct TestSession(Instant);

    impl SessionData for TestSession {
        fn created_at(&self) -> Instant {
            self.0
        }
    }

    #[tokio::test]
    async fn session_count_is_bounded() {
        let store = SessionStore::new(Duration::from_secs(60 * 60), 64);
        for _ in 0..1000 {
            store.add(TestSession(Instant::now())).await.unwrap();
        }
        let sessions = store.sessions.read().await;
        assert_eq!(sessions.by_id.len(), 64);
        assert_eq!(sessions.by_age.len(), 64);
    }

    #[tokio::test]
    async fn full_store_evicts_the_oldest_session() {
        let store = SessionStore::new(Duration::from_secs(60 * 60), 2);
        let now = Instant::now();
        let oldest = store.add(TestSession(now)).await.unwrap();
        let middle = store.add(TestSession(now + Duration::from_secs(1))).await.unwrap();
        let newest = store.add(TestSession(now + Duration::from_secs(2))).await.unwrap();

        assert!(store.get(&oldest).await.is_err());
        assert!(store.get(&middle).await.is_ok());
        assert!(store.get(&newest).await.is_ok());
    }

    #[tokio::test]
    async fn expired_sessions_are_not_returned_and_are_pruned_on_add() {
        let ttl = Duration::from_secs(60);
        let store = SessionStore::new(ttl, 64);
        let Some(expired_at) = Instant::now().checked_sub(ttl * 2) else {
            return;
        };
        let expired = store.add(TestSession(expired_at)).await.unwrap();
        assert!(store.get(&expired).await.is_err());

        let live = store.add(TestSession(Instant::now())).await.unwrap();
        assert!(store.get(&live).await.is_ok());
        let sessions = store.sessions.read().await;
        assert!(!sessions.by_id.contains_key(&expired));
        assert_eq!(sessions.by_age.len(), 1);
    }

    #[tokio::test]
    async fn removed_sessions_leave_no_age_entry() {
        let store = SessionStore::new(Duration::from_secs(60 * 60), 64);
        let session_id = store.add(TestSession(Instant::now())).await.unwrap();
        store.remove(&session_id).await.unwrap();
        assert!(store.remove(&session_id).await.is_err());
        assert!(store.sessions.read().await.by_age.is_empty());
    }
}
