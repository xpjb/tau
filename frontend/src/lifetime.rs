//! Worker idle TTL: never infer it from chat activity/title timestamps.
use std::{
    collections::HashMap,
    time::{Duration, Instant},
};
use tau_protocol::SessionStatus;

pub const IDLE_TTL: Duration = Duration::from_secs(60 * 60);
#[derive(Default)]
pub struct Lifetimes(HashMap<String, Instant>);
impl Lifetimes {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn update(&mut self, id: &str, status: SessionStatus, remaining_ms: Option<u64>) {
        self.0.remove(id);
        if status == SessionStatus::Idle
            && let Some(ms) = remaining_ms
        {
            self.0.insert(
                id.into(),
                Instant::now() + Duration::from_millis(ms).min(IDLE_TTL),
            );
        }
    }
    pub fn remaining(&self, id: &str) -> Option<Duration> {
        self.0
            .get(id)
            .map(|deadline| deadline.saturating_duration_since(Instant::now()))
    }
    pub fn meter(&self, id: &str, status: SessionStatus, connected: bool) -> (Option<f32>, u32) {
        if !connected {
            return (None, 0x82909f);
        }
        match status {
            SessionStatus::Idle => match self.remaining(id) {
                Some(time) => {
                    // Minute-sized steps; bounded texture variants, no animation loop.
                    let ratio = (time.as_secs_f32() / 60.).ceil() / 60.;
                    (
                        Some(ratio),
                        if time <= Duration::from_secs(300) {
                            0xfbbf24
                        } else {
                            0x67d4ff
                        },
                    )
                }
                None => (None, 0x82909f),
            },
            SessionStatus::Running | SessionStatus::Starting => (Some(1.), 0x4ade80),
            SessionStatus::Sleeping | SessionStatus::Error => (Some(0.), 0x82909f),
        }
    }
    pub fn details(&self, id: &str, status: SessionStatus, connected: bool) -> String {
        let state = if !connected {
            "Offline · remaining time unknown".into()
        } else {
            match status {
                SessionStatus::Idle => match self.remaining(id) {
                    Some(time) if !time.is_zero() => format!("{} min remaining", time.as_secs().div_ceil(60).max(1)),
                    Some(_) => "Idle TTL elapsed; awaiting worker state.\nQueued/paused work may keep it awake.".into(),
                    None => "Countdown unavailable from this daemon.\nAn updated daemon supplies the idle deadline.".into(),
                },
                SessionStatus::Running | SessionStatus::Starting => "Active · countdown starts when idle".into(),
                SessionStatus::Sleeping => "Worker asleep".into(),
                SessionStatus::Error => "Worker unavailable".into(),
            }
        };
        format!(
            "Worker idle TTL · 1 hour\n{state}\nSynced with the existing 20s heartbeat.\nExpiry sleeps the worker; chat history is kept."
        )
    }
}
