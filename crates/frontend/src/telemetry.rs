use crate::{controller::Controller, net::health::Phase};
use living_telemetry::{Config, Field};
use std::{
    path::PathBuf,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const DEFAULT_ENDPOINT: &str = "http://vibe.tail8689b0.ts.net:4318/v1/report";
static STATE_STARTED: OnceLock<Instant> = OnceLock::new();
static LAST_STATE_US: AtomicU64 = AtomicU64::new(0);

pub use living_telemetry::run_watchdog_from_args;
pub(crate) use living_telemetry::report_panic;

pub(crate) fn init(data_dir: PathBuf) -> Result<(), String> {
    let endpoint = match std::env::var("TAU_TELEMETRY_ENDPOINT") {
        Ok(value) if ["", "0", "off", "none"].iter().any(|disabled| value.trim().eq_ignore_ascii_case(disabled)) => None,
        Ok(value) => Some(value.trim().to_string()),
        Err(_) => Some(DEFAULT_ENDPOINT.to_string()),
    };
    let mut config = Config::new("tau2", env!("CARGO_PKG_VERSION"), data_dir);
    config.endpoint = endpoint;
    living_telemetry::init(config)
}

pub(crate) fn shutdown() {
    living_telemetry::shutdown();
}

pub(crate) fn chat_key(id: &str) -> u64 {
    let digest = blake3::hash(id.as_bytes());
    let mut key = [0; 8];
    key.copy_from_slice(&digest.as_bytes()[..8]);
    u64::from_le_bytes(key)
}

pub(crate) fn set_refresh_rate_millihertz(refresh_rate: Option<u32>) {
    let refresh_rate = refresh_rate.filter(|value| (20_000..=500_000).contains(value)).unwrap_or(60_000);
    let nanos = 1_000_000_000_000u64 / u64::from(refresh_rate);
    living_telemetry::set_frame_budget(Duration::from_nanos(nanos.max(1)));
}

pub(crate) fn update_app_state(controller: &Controller, window_focused: bool, window_visible: bool, force: bool) {
    let now_us = (STATE_STARTED.get_or_init(Instant::now).elapsed().as_micros().min(u128::from(u64::MAX)) as u64).saturating_add(1);
    let last_us = LAST_STATE_US.load(Ordering::Acquire);
    if !force && last_us > 0 && now_us.saturating_sub(last_us) < 250_000 {
        return;
    }
    LAST_STATE_US.store(now_us, Ordering::Release);
    let phase = match controller.health.phase {
        Phase::Offline => "offline",
        Phase::Connecting => "connecting",
        Phase::Connected => "connected",
        Phase::Disconnected => "disconnected",
        Phase::Blocked => "blocked",
    };
    let running_sessions = controller.account.sessions.iter().filter(|session| session.status == tau_net::SessionStatus::Running).count();
    let pending_messages = controller.chats.values().map(|chat| chat.local.pending.len()).sum::<usize>();
    let active_downloads = controller.downloads.values().filter(|download| !download.status.done).count();
    let selected_status = controller.account.selected.as_ref()
        .and_then(|selected| controller.account.sessions.iter().find(|session| &session.id == selected))
        .map(|session| format!("{:?}", session.status))
        .unwrap_or_else(|| "none".to_string());
    let metrics = &controller.native_metrics;
    let selected = controller.selected();
    living_telemetry::set_state(vec![
        Field::new("connection_phase", phase),
        Field::new("connected", controller.epoch.is_some()),
        Field::new("session_count", controller.account.sessions.len()),
        Field::new("loaded_chats", controller.chats.len()),
        Field::new("running_sessions", running_sessions),
        Field::new("selected_status", selected_status),
        Field::new("selected_chat_key", controller.account.selected.as_deref().map_or(0, chat_key)),
        Field::new("selected_loaded", selected.is_some()),
        Field::new("selected_feed_messages", selected.map_or(0, |chat| chat.feed.order.len())),
        Field::new("selected_feed_events", selected.map_or(0, |chat| chat.feed.events.len())),
        Field::new("selected_feed_bodies", selected.map_or(0, |chat| chat.feed.bodies.len())),
        Field::new("selected_feed_incomplete", selected.map_or(0, |chat| chat.feed.incomplete.len())),
        Field::new("selected_queue_items", selected.map_or(0, |chat| chat.feed.queue.requests.len())),
        Field::new("selected_local_pending", selected.map_or(0, |chat| chat.local.pending.len())),
        Field::new("selected_synchronized", selected.is_some_and(|chat| chat.feed.synchronized)),
        Field::new("selected_loading", selected.is_some_and(|chat| chat.feed.loading)),
        Field::new("selected_opening", selected.is_some_and(|chat| chat.feed.opening)),
        Field::new("selected_has_before", selected.is_some_and(|chat| chat.feed.before.is_some())),
        Field::new("selected_has_after", selected.is_some_and(|chat| chat.feed.after)),
        Field::new("pending_messages", pending_messages),
        Field::new("pending_controls", controller.account.pending_controls.len()),
        Field::new("restore_reviews", controller.restore_reviews.len()),
        Field::new("active_downloads", active_downloads),
        Field::new("native_active_streams", metrics.active_streams),
        Field::new("native_quic_rtt_ms", metrics.quic_rtt_ms),
        Field::new("native_quic_lost_packets", metrics.quic_lost_packets),
        Field::new("window_focused", window_focused),
        Field::new("window_visible", window_visible),
        Field::new("frame_budget_us", living_telemetry::frame_budget().as_micros() as u64),
    ]);
}
