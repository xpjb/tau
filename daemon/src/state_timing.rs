//! Opt-in local diagnostics: no SQL, paths, request bodies or wire fields.
//! Clock reads are disabled unless the `taud::db` debug target is enabled.
use std::time::Instant;

struct Clock { queued: Instant, acquired: Option<Instant>, working: Option<Instant> }
pub(crate) struct Timing { pool: &'static str, clock: Option<Clock>, outcome: &'static str }
impl Timing {
    pub fn new(pool: &'static str) -> Self {
        Self { pool, clock: tracing::enabled!(target:"taud::db",tracing::Level::DEBUG)
            .then(|| Clock { queued:Instant::now(),acquired:None,working:None }), outcome:"cancelled" }
    }
    pub fn acquired(&mut self) { if let Some(c)=&mut self.clock {c.acquired=Some(Instant::now());} }
    pub fn working(&mut self) {
        if let Some(c)=&mut self.clock {
            c.working=Some(Instant::now());
            tracing::debug!(target:"taud::db",pool=self.pool,phase="start","database work started");
        }
    }
    pub fn finished(&mut self, success: bool) { self.outcome=if success {"ok"} else {"error"}; }
}
impl Drop for Timing {
    fn drop(&mut self) {
        let Some(c)=&self.clock else {return;}; let now=Instant::now();
        let acquired=c.acquired.unwrap_or(now);let working=c.working.unwrap_or(now);
        tracing::debug!(target:"taud::db",pool=self.pool,phase="complete",outcome=self.outcome,
            wait_us=acquired.duration_since(c.queued).as_micros() as u64,
            dispatch_us=working.duration_since(acquired).as_micros() as u64,
            work_us=now.duration_since(working).as_micros() as u64,
            "database access timing");
    }
}
