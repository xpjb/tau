//! Bounded local samples of opt-in daemon timing events, without a wire change.
use std::{collections::BTreeMap,sync::{Arc,Mutex,atomic::{AtomicBool,AtomicU64,Ordering}},time::{Duration,Instant}};
use serde_json::{Value,json};
use tracing::{Event,Subscriber,field::{Field,Visit}};
use tracing_subscriber::{Layer,layer::Context,prelude::*};

pub fn distribution(values:&[u64])->Value {
    if values.is_empty() {return json!({"count":0});}
    let mut sorted=values.to_vec();sorted.sort_unstable();
    let ms=|v:u64|v as f64/1000.;
    json!({"count":sorted.len(),"p50_ms":ms(sorted[(sorted.len()-1)/2]),
        "p95_ms":ms(sorted[(sorted.len()*95).div_ceil(100)-1]),"max_ms":ms(*sorted.last().unwrap()),
        "total_ms":values.iter().map(|&v|ms(v)).sum::<f64>()})
}
#[derive(Default)]
struct Samples {wait:Vec<u64>,dispatch:Vec<u64>,work:Vec<u64>,outcomes:BTreeMap<String,u64>}
impl Samples {
    fn add(&mut self,event:&Fields) {
        *self.outcomes.entry(event.outcome.clone()).or_default()+=1;
        if self.wait.len()<20_000 {self.wait.push(event.wait);self.dispatch.push(event.dispatch);self.work.push(event.work);}
    }
    fn report(&self)->Value {json!({"wait":distribution(&self.wait),"dispatch":distribution(&self.dispatch),
        "work":distribution(&self.work),"outcomes":self.outcomes,"sample_cap":20_000})}
}
#[derive(Default)]
struct Data {pools:BTreeMap<String,Samples>,admission:Samples,content_failures:Vec<String>}
#[derive(Default)]
struct Inner {data:Mutex<Data>,pause_ms:AtomicU64,pause_started:AtomicBool,pause_actual_us:AtomicU64}
#[derive(Clone,Default)]
pub struct Recorder(Arc<Inner>);
impl Recorder {
    pub fn install()->Self {
        let recorder=Self::default();
        let filter=tracing_subscriber::filter::Targets::new()
            .with_target("taud::db",tracing::Level::DEBUG).with_target("taud::control_admission",tracing::Level::DEBUG)
            .with_target("tau::content",tracing::Level::DEBUG).with_target("log",tracing::Level::DEBUG);
        let filter=if std::env::var_os("TAU_NATIVE_TRACE").is_some() {
            filter.with_target("iroh_quinn_proto::connection",tracing::Level::TRACE)
                .with_target("tau_native_watch",tracing::Level::TRACE).with_target("log",tracing::Level::TRACE)
        } else {filter};
        tracing_subscriber::registry().with(recorder.clone().with_filter(filter)).try_init().unwrap();
        super::native_trace::init();
        log::debug!(target:"tau::content","pressure observer self-check");
        assert_eq!(recorder.take()["content_failures"][0],"pressure observer self-check","Content diagnostic bridge must be active");
        recorder
    }
    // Only this test subscriber sleeps: pause a real writer while it owns its
    // application lock, not a competing SQLite writer with different BUSY rules.
    pub fn pause_next_writer(&self,milliseconds:u64) {self.0.pause_ms.store(milliseconds,Ordering::SeqCst);}
    pub fn pause_started(&self)->bool {self.0.pause_started.load(Ordering::SeqCst)}
    pub fn pause_actual_us(&self)->u64 {self.0.pause_actual_us.load(Ordering::SeqCst)}
    pub fn take(&self)->Value {
        let data=std::mem::take(&mut *self.0.data.lock().unwrap());
        json!({"database":data.pools.iter().map(|(key,value)|(key.clone(),value.report())).collect::<BTreeMap<_,_>>(),
            "admission":data.admission.report(),"content_failures":data.content_failures})
    }
}
#[derive(Default)]
struct Fields {pool:String,phase:String,outcome:String,log_target:String,message:String,wait:u64,dispatch:u64,work:u64}
impl Visit for Fields {
    fn record_u64(&mut self,field:&Field,value:u64) {match field.name() {"wait_us"=>self.wait=value,"dispatch_us"=>self.dispatch=value,"work_us"=>self.work=value,_=>{}}}
    fn record_str(&mut self,field:&Field,value:&str) {match field.name() {"pool"=>self.pool=value.into(),"phase"=>self.phase=value.into(),"outcome"=>self.outcome=value.into(),"log.target"=>self.log_target=value.into(),_=>{}}}
    fn record_debug(&mut self,field:&Field,value:&dyn std::fmt::Debug) {if field.name()=="message" {self.message=format!("{value:?}");}}
}
impl<S:Subscriber> Layer<S> for Recorder {
    fn on_event(&self,event:&Event<'_>,_:Context<'_,S>) {
        let mut fields=Fields::default();event.record(&mut fields);
        let target=if fields.log_target.is_empty() {event.metadata().target()} else {&fields.log_target};
        if target=="tau_native_watch" || target.starts_with("iroh_quinn_proto::connection")
            && ["PTO fired", "keep-alive", "timeout", "handshake", "established", "keys", "CRYPTO", "TimedOut", "closing connection"]
                .iter().any(|text| fields.message.contains(text)) {
            super::native_trace::event(&format!("{target}: {}",fields.message));
        }
        if fields.phase=="start" {
            if fields.pool=="writer" {
                let milliseconds=self.0.pause_ms.swap(0,Ordering::SeqCst);
                if milliseconds>0 {
                    self.0.pause_started.store(true,Ordering::SeqCst);let at=Instant::now();
                    std::thread::sleep(Duration::from_millis(milliseconds));
                    self.0.pause_actual_us.store(at.elapsed().as_micros() as u64,Ordering::SeqCst);
                }
            }
            return;
        }
        let mut data=self.0.data.lock().unwrap();
        if fields.log_target=="tau::content" && data.content_failures.len()<200 {data.content_failures.push(fields.message.clone());}
        match event.metadata().target() {
            "taud::db"=>data.pools.entry(fields.pool.clone()).or_default().add(&fields),
            "taud::control_admission"=>data.admission.add(&fields),
            _=>{}
        }
    }
}
