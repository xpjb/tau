//! Opt-in, fixture-only packet/timer timeline. Never log payloads or credentials.
use std::{sync::OnceLock, time::Instant};
static START: OnceLock<Instant> = OnceLock::new();
struct Logger;
static LOGGER: Logger = Logger;
impl log::Log for Logger {
    fn enabled(&self, meta: &log::Metadata<'_>) -> bool {
        meta.target().starts_with("iroh_quinn_proto::connection") || meta.target()=="tau_native_watch"
    }
    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) { return; }
        let message=record.args().to_string();
        if record.target()=="tau_native_watch" || ["PTO fired", "keep-alive", "idle timeout", "packets lost", "TimedOut"]
            .iter().any(|s|message.contains(s)) { event(&format!("{} {message}",record.target())); }
    }
    fn flush(&self) {}
}
pub(super) fn init() {
    if std::env::var_os("TAU_NATIVE_TRACE").is_some() {
        let _=START.set(Instant::now());
        let _=log::set_logger(&LOGGER);
        log::set_max_level(log::LevelFilter::Trace);
    }
}
pub(super) fn event(message: &str) {
    if let Some(start)=START.get() { eprintln!("native-trace +{:.3}s {message}",start.elapsed().as_secs_f64()); }
}
pub(super) fn packet(kind: &str, direction: usize, bytes: usize) {
    if START.get().is_some() { event(&format!("UDP {kind} {} {bytes} bytes",if direction==0 {"client→server"} else {"server→client"})); }
}
