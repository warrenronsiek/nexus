// @feature runtime
// @spec docs/features/runtime.md
// Opaque sessions and admission share process ownership and are discarded on restart.
use super::credentials::Credentials;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use uuid::Uuid;

pub(super) const SESSION_SECONDS: u64 = 8 * 60 * 60;
const MAX_SESSIONS: usize = 256;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const MAX_LOGIN_ATTEMPTS: usize = 12;

pub(super) struct Auth {
    pub(super) credentials: Option<Credentials>,
    sessions: Mutex<HashMap<[u8; 32], Instant>>,
    attempts: Mutex<VecDeque<Instant>>,
    hash_slots: Arc<Semaphore>,
}
impl Auth {
    pub(super) fn new(credentials: Option<Credentials>) -> Self {
        Self {
            credentials,
            sessions: Mutex::default(),
            attempts: Mutex::default(),
            hash_slots: Arc::new(Semaphore::new(2)),
        }
    }
    pub(super) fn admit_login(&self, now: Instant) -> Option<OwnedSemaphorePermit> {
        let permit = self.hash_slots.clone().try_acquire_owned().ok()?;
        let mut attempts = self.attempts.lock().ok()?;
        while attempts
            .front()
            .is_some_and(|attempt| now.duration_since(*attempt) >= LOGIN_WINDOW)
        {
            attempts.pop_front();
        }
        if attempts.len() >= MAX_LOGIN_ATTEMPTS {
            return None;
        }
        attempts.push_back(now);
        Some(permit)
    }
    pub(super) fn create_session(&self, now: Instant) -> Option<String> {
        let mut sessions = self.sessions.lock().ok()?;
        sessions.retain(|_, expires| *expires > now);
        if sessions.len() >= MAX_SESSIONS {
            return None;
        }
        let token = format!("{}{}", Uuid::new_v4().simple(), Uuid::new_v4().simple());
        sessions.insert(
            *blake3::hash(token.as_bytes()).as_bytes(),
            now + Duration::from_secs(SESSION_SECONDS),
        );
        Some(token)
    }
    pub(super) fn authenticated(&self, token: Option<&str>, now: Instant) -> bool {
        let Some(token) = token else {
            return false;
        };
        let Ok(mut sessions) = self.sessions.lock() else {
            return false;
        };
        sessions.retain(|_, expires| *expires > now);
        sessions.contains_key(blake3::hash(token.as_bytes()).as_bytes())
    }
    pub(super) fn revoke(&self, token: Option<&str>) {
        if let (Some(token), Ok(mut sessions)) = (token, self.sessions.lock()) {
            sessions.remove(blake3::hash(token.as_bytes()).as_bytes());
        }
    }
}
