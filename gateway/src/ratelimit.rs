//! Sliding-window rate limiter keyed by user.

use std::{
    collections::{HashMap, VecDeque},
    sync::Mutex,
    time::{Duration, Instant},
};

use uuid::Uuid;

pub struct SlidingWindow {
    limit: usize,
    window: Duration,
    hits: Mutex<HashMap<Uuid, VecDeque<Instant>>>,
}

impl SlidingWindow {
    pub fn new(limit: usize, window: Duration) -> Self {
        Self {
            limit,
            window,
            hits: Mutex::default(),
        }
    }

    /// Records a hit for `key` and returns whether it is within the limit.
    pub fn allow(&self, key: Uuid) -> bool {
        let now = Instant::now();
        let mut hits = self.hits.lock().expect("rate limit lock poisoned");
        let queue = hits.entry(key).or_default();
        while queue
            .front()
            .is_some_and(|at| now.duration_since(*at) >= self.window)
        {
            queue.pop_front();
        }
        if queue.len() >= self.limit {
            false
        } else {
            queue.push_back(now);
            true
        }
    }
}
