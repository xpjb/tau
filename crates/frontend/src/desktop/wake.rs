//! One queued OS wake per UI drain, not one Windows message per network chunk.
//! Clear BEFORE draining: arrivals during/after a drain must queue the next one.
use crate::net::Wake;
use std::sync::{Arc, atomic::{AtomicBool, AtomicU64, Ordering}};

pub(crate) struct WakeGate {
    pending: AtomicBool,
    received: AtomicU64,
    posted: AtomicU64,
    notify: Wake,
}
impl WakeGate {
    pub fn new(notify: Wake) -> Arc<Self> {
        Arc::new(Self { pending: AtomicBool::new(false), received: AtomicU64::new(0),
            posted: AtomicU64::new(0), notify })
    }
    pub fn wake(&self) {
        self.received.fetch_add(1, Ordering::Relaxed);
        if !self.pending.swap(true, Ordering::AcqRel) {
            self.posted.fetch_add(1, Ordering::Relaxed);
            (self.notify)();
        }
    }
    pub fn callback(self: &Arc<Self>) -> Wake {
        let gate = self.clone();
        Arc::new(move || gate.wake())
    }
    pub fn begin_update(&self) -> (u64, u64) {
        let received = self.received.swap(0, Ordering::Relaxed);
        let posted = self.posted.swap(0, Ordering::Relaxed);
        self.pending.store(false, Ordering::Release);
        (received, posted)
    }
}

#[cfg(test)]
#[path = "../../tests/unit/desktop/wake.rs"]
mod tests;
