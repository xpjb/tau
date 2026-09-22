//! Presentation of source timestamps. Never timestamp old history on arrival.
use chrono::{DateTime, Local, Utc};
use tau_protocol::Event;

pub fn now_ms() -> Option<u64> {
    Utc::now().timestamp_millis().try_into().ok()
}

pub fn event_ms(event: &Event) -> Option<u64> {
    event.timestamp_ms.or_else(|| {
        DateTime::parse_from_rfc3339(event.timestamp.as_deref()?)
            .ok()?
            .timestamp_millis()
            .try_into()
            .ok()
    })
}

pub fn label(timestamp: Option<u64>) -> String {
    timestamp
        .and_then(|ms| i64::try_from(ms).ok())
        .and_then(DateTime::<Utc>::from_timestamp_millis)
        .map(|time| {
            time.with_timezone(&Local)
                .format("%b %-d · %H:%M:%S")
                .to_string()
        })
        .unwrap_or_else(|| "Time unavailable".into())
}
