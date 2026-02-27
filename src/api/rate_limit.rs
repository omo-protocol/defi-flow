use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use tokio::sync::Mutex;

const MAX_REQUESTS: usize = 10;
const WINDOW: Duration = Duration::from_secs(60);

pub struct RateLimiter {
    requests: Mutex<HashMap<String, VecDeque<Instant>>>,
}

impl RateLimiter {
    pub fn new() -> Self {
        Self {
            requests: Mutex::new(HashMap::new()),
        }
    }

    /// Returns Ok(()) if allowed, Err with seconds until next slot if rate limited.
    pub async fn check(&self, user_id: &str) -> Result<(), u64> {
        let mut map = self.requests.lock().await;
        let now = Instant::now();
        let entry = map.entry(user_id.to_string()).or_default();

        // Evict expired entries
        while entry.front().is_some_and(|t| now.duration_since(*t) > WINDOW) {
            entry.pop_front();
        }

        if entry.len() >= MAX_REQUESTS {
            let oldest = entry.front().unwrap();
            let retry_after = WINDOW.as_secs() - now.duration_since(*oldest).as_secs();
            return Err(retry_after.max(1));
        }

        entry.push_back(now);
        Ok(())
    }
}
