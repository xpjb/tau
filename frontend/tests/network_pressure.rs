#![cfg(unix)]
//! Real controller/transport/cache + daemon + local provider, with seeded link
//! faults and mixed work. Run via nextest; each case gets its own process/observer.
use std::{collections::{BTreeMap,BTreeSet},net::SocketAddr,path::PathBuf,sync::{Arc,atomic::{AtomicU64,Ordering}},time::{Duration,Instant}};
use axum::{Json,Router,extract::State,response::IntoResponse,routing::post};
use futures_util::{SinkExt,StreamExt};
use serde_json::{Value,json};
use tau_frontend::{controller::Controller,store::{Settings,Store}};
use tau_protocol::*;
use tokio::net::{TcpListener,TcpStream};
use tokio_tungstenite::{connect_async,tungstenite::{Message,client::IntoClientRequest}};
#[path="support/pressure_link.rs"] #[allow(dead_code)] mod pressure_link;
#[path="support/pressure_metrics.rs"] mod pressure_metrics;
use pressure_link::{Link,Profile,Proxy,draw};
use pressure_metrics::{Recorder,distribution};
const TOKEN:&str="isolated-network-pressure-fixture-only";
const FILE_BYTES:usize=1024*1024;
const DB_HOLD_MS:u64=400;

struct Model {calls:AtomicU64,gate:tokio::sync::Notify,root:PathBuf,answer:String}
async fn reply(State(model):State<Arc<Model>>,Json(_):Json<Value>)->impl IntoResponse {
    let n=model.calls.fetch_add(1,Ordering::SeqCst);
    assert!(n<12,"Unexpected provider replay");
    if n==0 {model.gate.notified().await;}
    let delta=if n==0 {
        let mut calls=(0..3).map(|n|json!({"index":n,"id":format!("file-{n}"),"type":"function","function":{"name":"send_file",
            "arguments":json!({"path":model.root.join(format!("outbox/file-{n}.bin"))}).to_string()}})).collect::<Vec<_>>();
        calls.push(json!({"index":3,"id":"read-pressure","type":"function","function":{"name":"read","arguments":json!({"path":model.root.join("read.txt")}).to_string()}}));
        json!({"tool_calls":calls})
    } else {json!({"content":format!("Reply {n}: {}",model.answer)})};
    let text=format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"index":0,"delta":delta}]}),
        json!({"choices":[{"index":0,"delta":{},"finish_reason":if n==0 {"tool_calls"} else {"stop"}}],"usage":{"total_tokens":100}}));
    ([("content-type","text/event-stream")],text)
}
struct Fixture {
    clients:[Controller;2],
    _locals:[tempfile::TempDir;2],
    source:tempfile::TempDir,
    proxies:[Proxy;2],
    link:Arc<Link>,
    model:Arc<Model>,
    bytes:Vec<u8>,
    tasks:Vec<tokio::task::JoinHandle<()>>,
}
impl Drop for Fixture {fn drop(&mut self) {for task in &self.tasks {task.abort();}}}
impl Fixture {
    async fn new(profile:Profile,seed:u64)->Self {
        let source=tempfile::tempdir().unwrap();std::fs::create_dir(source.path().join("outbox")).unwrap();
        let mut bytes=vec![0;FILE_BYTES];blake3::Hasher::new().update(&seed.to_le_bytes()).finalize_xof().fill(&mut bytes);
        for n in 0..3 {std::fs::write(source.path().join(format!("outbox/file-{n}.bin")),&bytes).unwrap();}
        std::fs::write(source.path().join("read.txt"),"VISIBLE-PRESSURE-TOOL ".repeat(700)).unwrap();
        let model=Arc::new(Model {calls:AtomicU64::new(0),gate:tokio::sync::Notify::new(),root:source.path().into(),
            answer:(0..180).map(|n|format!("{} café 😀\n",blake3::hash(format!("{seed}-{n}").as_bytes()).to_hex())).collect()});
        let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let address=listener.local_addr().unwrap();
        let app=Router::new().route("/{*path}",post(reply).get(||async {
            Json(json!({"data":[{"id":tau_protocol::settings::Settings::default().agent.model.model_id,"context_length":200_000}]}))
        })).with_state(model.clone());
        let provider=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
        let mut settings=tau_protocol::settings::Settings::default();settings.agent.load_agents_files=false;
        settings.agent.retry.enabled=false;settings.daemon.generate_titles=false;settings.daemon.idle_timeout_seconds=0;
        let p=settings.providers.get_mut("openai-codex").unwrap();p.api=tau_protocol::settings::Api::ChatCompletions;
        p.base_url=format!("http://{address}");p.web_search=false;
        std::fs::write(source.path().join("settings.json"),serde_json::to_vec(&settings).unwrap()).unwrap();
        let auth=source.path().join("auth.json");std::fs::write(&auth,r#"{"openai-codex":{"type":"api_key","key":"local-pressure-fixture"}}"#).unwrap();
        {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(auth,std::fs::Permissions::from_mode(0o600)).unwrap();}
        let tcp=std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let udp=std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
        let config=taud::Config {bind:tcp,transfer_bind:match udp {SocketAddr::V4(a)=>a,_=>unreachable!()},transfer_bind_v6:None,
            token:Arc::from(TOKEN),settings_path:source.path().join("settings.json"),import_pi_dir:None,codex_auth_source:None,
            cwd:source.path().into(),database_path:source.path().join("tau.sqlite3"),telemetry_path:source.path().join("crashes.jsonl"),
            attachment_root:source.path().join("outbox"),upload_root:source.path().join("uploads")};
        let daemon=tokio::spawn(async move {taud::run(config).await.unwrap()});
        tokio::time::timeout(Duration::from_secs(10),async {loop {if TcpStream::connect(tcp).await.is_ok() {break;}tokio::time::sleep(Duration::from_millis(10)).await;}}).await.unwrap();
        let link=Link::profile(profile,seed);
        let proxies=[Proxy::new("127.0.0.2",tcp,udp,link.clone()).await,Proxy::new("127.0.0.3",tcp,udp,link.clone()).await];
        let locals=[tempfile::tempdir().unwrap(),tempfile::tempdir().unwrap()];
        let clients=std::array::from_fn(|n| {
            let store=Store::open(locals[n].path().into()).unwrap();
            store.put("","settings",&Settings {server_url:format!("http://{}",proxies[n].address),token:TOKEN.into()}).unwrap();
            Controller::new(store,Arc::new(||{})).unwrap()
        });
        Self {clients,_locals:locals,source,proxies,link,model,bytes,tasks:vec![daemon,provider]}
    }
    fn prompt(&mut self,index:usize,text:&str)->String {
        let c=&mut self.clients[index];c.draft(text.into()).unwrap();c.send_prompt().unwrap();
        c.selected().unwrap().local.pending.last().unwrap().request.id.clone()
    }
}
#[derive(Default)]
struct Observed {
    notices:Vec<String>,transport:BTreeSet<String>,poll_us:Vec<u64>,disconnects:[u64;2],epochs:[Option<u64>;2],
}
impl Observed {
    fn poll(&mut self,f:&mut Fixture) {
        for (n,c) in f.clients.iter_mut().enumerate() {
            let at=Instant::now();c.poll();if self.poll_us.len()<20_000 {self.poll_us.push(at.elapsed().as_micros() as u64);}
            if let Some(notice)=c.notice.take() && self.notices.len()<200 {self.notices.push(format!("client {n}: {notice}"));}
            if let Some(detail)=&c.transport_error {self.transport.insert(detail.clone());}
            if self.epochs[n].is_some() && c.epoch.is_none() {self.disconnects[n]+=1;}
            self.epochs[n]=c.epoch;
        }
    }
}
async fn until(f:&mut Fixture,o:&mut Observed,label:&str,predicate:impl Fn(&Fixture)->bool) {
    let at=Instant::now();loop {o.poll(f);if predicate(f) {return;}
        assert!(at.elapsed()<Duration::from_secs(40),"{label} stalled; notices={:?}; A={} B={}",o.notices,f.clients[0].diagnostics(),f.clients[1].diagnostics());
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
async fn recover_bulk(f:&mut Fixture,o:&mut Observed,recorder:&Recorder,restored:Instant,before:[Option<u64>;2],seed:u64)->Value {
    // Liveness is not total file time. Keep a strict no-progress deadline, and
    // separately budget useful throughput for every competing unfinished file.
    let mut remaining=0u64;let mut watched=BTreeMap::new();
    for (n,c) in f.clients.iter().enumerate() {for (key,d) in &c.downloads {if !d.status.done {
        remaining+=(FILE_BYTES as u64).saturating_sub(d.status.transferred);
        watched.insert((n,key.clone()),(d.status.transferred,restored,None::<u64>,0u64));
    }}}
    const MIN_GOODPUT:u64=32*1024;const NO_PROGRESS_MS:u64=15_000;
    let budget_ms=10_000+(remaining*1000).div_ceil(MIN_GOODPUT);let mut socket_ms=None;
    loop {
        o.poll(f);let now=Instant::now();let mut done=true;let mut stalled=false;
        if socket_ms.is_none() && f.clients.iter().enumerate().all(|(n,c)|c.epoch.is_some()&&c.epoch!=before[n]) {
            socket_ms=Some(restored.elapsed().as_millis());
        }
        for ((n,key),(previous,last,first,max_gap)) in &mut watched {
            let d=&f.clients[*n].downloads[key];
            if !d.status.done || d.status.transferred!=*previous {
                *max_gap=(*max_gap).max(now.duration_since(*last).as_millis() as u64);
            }
            if d.status.transferred>*previous || d.status.done {
                first.get_or_insert(now.duration_since(restored).as_millis() as u64);*last=now;*previous=d.status.transferred;
            }
            if !d.status.done {done=false;stalled|=now.duration_since(*last).as_millis()>u128::from(NO_PROGRESS_MS);}
        }
        let progress=watched.iter().map(|((n,key),(_,_,first,gap))| {
            let d=&f.clients[*n].downloads[key];json!({"client":n,"bytes":d.status.transferred,"done":d.status.done,"failure":d.status.failure,
                "first_application_progress_ms":first,"max_no_progress_ms":gap})
        }).collect::<Vec<_>>();
        let socket_stalled=socket_ms.is_none() && restored.elapsed()>Duration::from_secs(5);
        if stalled || socket_stalled || restored.elapsed().as_millis()>u128::from(budget_ms) {
            let cause=if socket_stalled {"socket reacquisition"} else if stalled {"no progress"} else {"bulk throughput budget"};
            persist_report("recovery",seed,&json!({"failure":cause,
                "progress":progress,"remaining_at_restore":remaining,"completion_budget_ms":budget_ms,
                "link":f.link.report(),"server":recorder.take(),"alerts":o.notices}));
            panic!("Recovery failed its {cause} guard; see report");
        }
        if done && socket_ms.is_some() {return json!({"progress":progress,"socket_reacquisition_ms":socket_ms,"reported_remaining_bytes_at_restore":remaining,
            "minimum_goodput_kib_s":MIN_GOODPUT/1024,"completion_budget_ms":budget_ms,"no_progress_deadline_ms":NO_PROGRESS_MS,
            "bulk_completion_after_restore_ms":restored.elapsed().as_millis()});}
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
#[derive(Default)]
struct Probe {latencies:Vec<u64>,errors:Vec<String>}
async fn probe(address:SocketAddr,session:String,count:u64,spacing_ms:u64)->Probe {
    let mut request=format!("ws://{address}/v1/ws").into_client_request().unwrap();
    request.headers_mut().insert("authorization",format!("Bearer {TOKEN}").parse().unwrap());
    let (mut socket,_)=connect_async(request).await.unwrap();
    let mut timer=tokio::time::interval(Duration::from_millis(spacing_ms));timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut pending=BTreeMap::new();let mut sent=0;let mut result=Probe::default();
    tokio::time::timeout(Duration::from_secs(30),async {
        while result.latencies.len()<count as usize {
            tokio::select! {
                _=timer.tick(),if sent<count && pending.len()<6=>{
                    let id=format!("probe-{sent}");let command=match sent%3 {
                        0=>ClientCommand::GetOperation {operation_id:"not-a-real-operation".into()},
                        1=>ClientCommand::ListSessions,
                        _=>ClientCommand::GetSession {session_id:session.clone()},
                    };
                    pending.insert(id.clone(),Instant::now());
                    socket.send(Message::Text(serde_json::to_string(&ClientRequest {id,command}).unwrap().into())).await.unwrap();sent+=1;
                }
                frame=socket.next()=>match frame.expect("Probe socket closed").unwrap() {
                    Message::Text(text)=>{
                        let v:Value=serde_json::from_str(&text).unwrap();
                        if v["type"]=="response" && let Some(at)=v["requestId"].as_str().and_then(|id|pending.remove(id)) {
                            result.latencies.push(at.elapsed().as_micros() as u64);
                            if v["ok"]!=true {result.errors.push(v.to_string());}
                        }
                    }
                    Message::Ping(bytes)=>{socket.send(Message::Pong(bytes)).await.unwrap();}
                    Message::Close(_)=>panic!("Probe closed unexpectedly"),
                    _=>{}
                }
            }
        }
    }).await.expect("Read probe timed out");
    let _=socket.close(None).await;result
}
async fn await_probe(f:&mut Fixture,o:&mut Observed,task:tokio::task::JoinHandle<Probe>)->Probe {
    while !task.is_finished() {o.poll(f);tokio::time::sleep(Duration::from_millis(10)).await;}
    task.await.unwrap()
}
fn persist_report(name:&str,seed:u64,report:&Value) {
    eprintln!("pressure-report: {}",serde_json::to_string_pretty(report).unwrap());
    if let Some(root)=std::env::var_os("TAU_PRESSURE_REPORT_DIR") {
        let root=PathBuf::from(root);std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join(format!("{name}-{seed}.json")),serde_json::to_vec_pretty(report).unwrap()).unwrap();
    }
}
async fn run(profile:Profile,default_seed:u64,outage:bool) {
    let seed=std::env::var("TAU_PRESSURE_SEED").map(|s|s.parse().expect("TAU_PRESSURE_SEED must be a u64")).unwrap_or(default_seed);
    let recorder=Recorder::install();let started=Instant::now();let mut f=Fixture::new(profile,seed).await;let mut o=Observed::default();
    until(&mut f,&mut o,"connection",|f|f.clients.iter().all(|c|c.epoch.is_some())).await;
    f.clients[0].new_chat().unwrap();
    until(&mut f,&mut o,"new chat",|f|f.clients[0].selected().is_some_and(|c|c.feed.synchronized)).await;
    let session=f.clients[0].account.selected.clone().unwrap();
    until(&mut f,&mut o,"catalogue",|f|f.clients[1].account.sessions.iter().any(|s|s.id==session)).await;
    f.clients[1].select(&session).unwrap();
    until(&mut f,&mut o,"second reader",|f|f.clients[1].selected().is_some_and(|c|c.feed.synchronized)).await;
    recorder.take();
    let baseline=tokio::spawn(probe(f.proxies[0].address,session.clone(),18,15));
    let baseline=await_probe(&mut f,&mut o,baseline).await;let baseline_db=recorder.take();
    let upload=f._locals[0].path().join("input.bin");std::fs::write(&upload,&f.bytes[..128*1024]).unwrap();
    f.clients[0].attach(&upload,None).unwrap();
    let mut authored=vec![f.prompt(0,&format!("Seed {seed}: inspect this upload and provide the files"))];
    until(&mut f,&mut o,"provider gate",|f|f.model.calls.load(Ordering::SeqCst)==1).await;
    for n in 0..3 {authored.push(f.prompt((draw(seed,n)&1) as usize,&format!("Queued seed {seed}, message {n}: café 😀")));}
    until(&mut f,&mut o,"queued inputs",|f|f.clients[0].selected().unwrap().feed.queue.requests.len()>=3).await;
    f.model.gate.notify_one();
    until(&mut f,&mut o,"file metadata",|f|f.clients.iter().all(|c|c.selected().unwrap().feed.events.values().filter(|e|e.attachment.is_some()).count()>=3)).await;
    let entries=f.clients[0].selected().unwrap().feed.events.values().filter_map(|e|e.attachment.as_ref().map(|a|(a.file_name.clone(),e.entry_id.clone()))).collect::<BTreeMap<_,_>>();
    let file_a=entries["file-0.bin"].clone();let file_b=entries["file-1.bin"].clone();let unread=entries["file-2.bin"].clone();
    let key_a=Controller::download_key(&session,&file_a);let key_b=Controller::download_key(&session,&file_b);
    let bulk_started=Instant::now();
    recorder.pause_next_writer(DB_HOLD_MS);
    let path_a=f.clients[0].download(&session,&file_a,MAX_UPLOAD_BYTES as u64).unwrap();
    let path_b=f.clients[1].download(&session,&file_b,MAX_UPLOAD_BYTES as u64).unwrap();
    // The opt-in test observer pauses one real application writer, while read
    // connections remain free. This is explicitly injected, not production data.
    let db_path=f.source.path().join("tau.sqlite3");
    until(&mut f,&mut o,"injected writer pause",|_|recorder.pause_started()).await;
    let mut pending=vec![];let mut operation_ids=vec![];
    for n in 0..24 {
        let index=(draw(seed^0x2424,n)&1) as usize;let at=Instant::now();
        let id=f.clients[index].request(ClientCommand::RenameSession {session_id:session.clone(),title:format!("Seed {seed}, rename {n}")}).unwrap();
        operation_ids.push(id.clone());pending.push((index,id,at));
    }
    let query_task=tokio::spawn(probe(f.proxies[1].address,session.clone(),48,15));
    let cancel_at=64*1024+(draw(seed,99)%4)*32*1024;
    let mut writes=vec![];let mut cancelled=false;let mut resumed=false;let mut toggles=0;let mut toggle_at=Instant::now();
    let mut trace=vec![];
    let pressure_at=Instant::now();
    loop {
        o.poll(&mut f);
        pending.retain(|(n,id,at)| {if f.clients[*n].account.pending_controls.contains_key(id) {true} else {writes.push(at.elapsed().as_micros() as u64);false}});
        let status=&f.clients[0].downloads[&key_a].status;
        if !cancelled && !status.done && status.transferred>=cancel_at {
            trace.push(format!("cancel at {}",status.transferred));f.clients[0].cancel_download(&key_a).unwrap();cancelled=true;
        }
        if cancelled && !resumed && f.clients[0].downloads[&key_a].status.done {
            trace.push("resume original file".into());f.clients[0].download(&session,&file_a,MAX_UPLOAD_BYTES as u64).unwrap();resumed=true;
        }
        if toggles<8 && toggle_at.elapsed()>Duration::from_millis(100+draw(seed,toggles)%160) {
            let c=&mut f.clients[(draw(seed^0x4040,toggles)&1) as usize];let chat=c.chats.get_mut(&session).unwrap();
            chat.local.details_default=true;
            chat.local.expansion.insert("tool:read-pressure".into(),toggles%2==0);
            chat.local.expansion.insert("tool:read-pressure:Output".into(),toggles%2==0);
            c.save_chat(&session).unwrap();toggles+=1;toggle_at=Instant::now();
        }
        if pending.is_empty() && query_task.is_finished() && (resumed || f.clients[0].downloads[&key_a].status.done) {break;}
        assert!(pressure_at.elapsed()<Duration::from_secs(35),"Pressure stalled: {:?}, notices={:?}",pending,o.notices);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let queries=query_task.await.unwrap();
    let steady_disconnects=o.disconnects;let steady_transport=o.transport.clone();
    let mut recovery=json!(null);
    if outage {
        let before=[f.clients[0].epoch,f.clients[1].epoch];
        f.clients[1].download(&session,&file_a,MAX_UPLOAD_BYTES as u64).unwrap();
        f.link.blackhole.store(true,Ordering::Relaxed);trace.push("blackhole TCP/UDP for 8 seconds".into());
        authored.push(f.prompt(0,&format!("Seed {seed}: original intent across outage")));
        let at=Instant::now();
        while at.elapsed()<Duration::from_secs(8) {o.poll(&mut f);tokio::time::sleep(Duration::from_millis(10)).await;}
        f.link.blackhole.store(false,Ordering::Relaxed);let restored=Instant::now();
        recovery=recover_bulk(&mut f,&mut o,&recorder,restored,before,seed).await;
        recovery["injected_outage_ms"]=json!(8000);
    }
    until(&mut f,&mut o,"verified downloads",|f|f.clients[0].downloads[&key_a].status.done&&f.clients[1].downloads[&key_b].status.done).await;
    let bulk_ms=bulk_started.elapsed().as_millis();
    if f.clients[0].downloads[&key_a].status.failure.is_some() || f.clients[1].downloads[&key_b].status.failure.is_some() {
        let report=json!({"seed":seed,"profile":profile.name,"server":recorder.take(),"link":f.link.report(),"alerts":o.notices,
            "file_errors":[f.clients[0].downloads[&key_a].status.failure,f.clients[1].downloads[&key_b].status.failure],"trace":trace});
        persist_report(profile.name,seed,&report);panic!("File transfer failed; see pressure report");
    }
    until(&mut f,&mut o,"canonical inputs",|f|f.clients.iter().all(|c|c.selected().unwrap().local.pending.is_empty()
        && authored.iter().all(|id|c.selected().unwrap().feed.events.values().any(|e|e.origin.request_id.as_deref()==Some(id.as_str())&&e.role==EventRole::User)))).await;
    until(&mut f,&mut o,"native metrics",|f|f.clients.iter().all(|c|c.native_metrics.content_rx_bytes>=FILE_BYTES as u64&&c.health.min_max().is_some())).await;
    let source=rusqlite::Connection::open(&db_path).unwrap();
    let copies=authored.iter().map(|id|source.query_row("SELECT count(*) FROM entries WHERE session_id=?1 AND json_extract(data,'$.origin.requestId')=?2 AND json_extract(data,'$.message.role')='user'",rusqlite::params![session,id],|r|r.get::<_,u64>(0)).unwrap()).collect::<Vec<_>>();
    let receipts=operation_ids.iter().map(|id|source.query_row("SELECT response FROM operations WHERE id=?1",[id],|r|r.get::<_,Option<String>>(0)).unwrap()).collect::<Vec<_>>();
    let unread_parts:u64=source.query_row("SELECT count(*) FROM block_parts WHERE scope=?1 AND id=?2",rusqlite::params![session,format!("file:{unread}")],|r|r.get(0)).unwrap();
    let db=recorder.take();
    let report=json!({"schema":1,"profile":profile.name,"seed":seed,"outage_case":outage,"elapsed_ms":started.elapsed().as_millis(),
        "link":f.link.report(),"baseline":{"control_reads":distribution(&baseline.latencies),"server":baseline_db},
        "pressure":{"control_reads":distribution(&queries.latencies),"control_writes":distribution(&writes),"injected_writer_pause_ms":DB_HOLD_MS,"observed_pause_ms":recorder.pause_actual_us() as f64/1000.,
            "server":db,"controller_poll":distribution(&o.poll_us),"bulk_bytes":(2+usize::from(outage))*FILE_BYTES,"bulk_elapsed_ms":bulk_ms,
            "bulk_goodput_kib_s":((2+usize::from(outage))*FILE_BYTES) as f64/1024./(bulk_ms as f64/1000.)},
        "recovery":recovery,"steady_disconnects":steady_disconnects,"all_disconnects":o.disconnects,
        "steady_transport_issues":steady_transport,"all_transport_issues":o.transport,"alerts":o.notices,
        "query_errors":queries.errors,"baseline_errors":baseline.errors,"original_prompt_copies":copies,
        "control_receipts":receipts.len(),"provider_calls":f.model.calls.load(Ordering::SeqCst),"unread_file_parts":unread_parts,
        "native":[f.clients[0].native_metrics.clone(),f.clients[1].native_metrics.clone()],"trace":trace});
    persist_report(if outage {"recovery"} else {profile.name},seed,&report);
    assert!(o.notices.is_empty(),"Unexpected alerts; see pressure report: {:?}",o.notices);
    assert!(baseline.errors.is_empty()&&queries.errors.is_empty(),"Control query failure");
    assert_eq!(steady_disconnects,[0,0],"Short stalls/loss must not cause socket churn");
    assert!(steady_transport.is_empty(),"Unnecessary transport errors under bounded jitter/loss");
    assert_eq!(writes.len(),24);assert!(receipts.iter().all(|s|s.as_ref().is_some_and(|v|serde_json::from_str::<Value>(v).unwrap()["ok"]==true)));
    assert!(copies.iter().all(|n|*n==1),"Original messages must not disappear or be reexecuted");assert_eq!(unread_parts,0);
    assert!(cancelled&&resumed,"The workload must actually exercise cancellation/resume");
    assert!(f.clients[0].downloads[&key_a].status.failure.is_none()&&f.clients[1].downloads[&key_b].status.failure.is_none());
    assert_eq!(std::fs::read(path_a).unwrap(),f.bytes);assert_eq!(std::fs::read(path_b).unwrap(),f.bytes);
    assert!(f.clients.iter().all(|c|c.native_metrics.integrity_failures==0));
    assert_eq!(db["admission"]["outcomes"]["timeout"].as_u64().unwrap_or(0),0,"Unexpected admission timeout");
    assert_eq!(db["admission"]["outcomes"]["overflow"].as_u64().unwrap_or(0),0,"Bounded workload must not overflow");
    assert!(db["database"]["writer"]["work"]["max_ms"].as_f64().unwrap()>100.,"The injected DB stall must be observed");
    assert!(db["database"]["reader"]["wait"]["max_ms"].as_f64().unwrap()<200.,"Short reads regressed behind the writer");
    assert!(*queries.latencies.iter().max().unwrap()<if profile.name=="normal" {300_000} else {4_000_000},"Read latency budget exceeded");
    assert!(*writes.iter().max().unwrap()<5_000_000,"Control write starved");
    if outage {
        assert!(o.disconnects.iter().all(|n|*n==1),"One real outage should cause one replacement per socket");
        assert!(recovery["socket_reacquisition_ms"].as_u64().unwrap()<5000,"Unnecessary socket reacquisition delay");
        assert!(recovery["progress"].as_array().unwrap().iter().all(|p|p["failure"].is_null()),"A bounded outage must not corrupt the resumed files");
        assert_eq!(std::fs::read(f.clients[1].attachment_path(&session,&file_a)).unwrap(),f.bytes);
    } else {assert!(f.clients.iter().all(|c|c.native_metrics.connections==1),"Healthy QUIC connection was unnecessarily replaced");}
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn normal_link_pressure_has_no_alerts_or_read_write_head_of_line_blocking() {run(Profile::NORMAL,7,false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn seeded_dodgy_link_keeps_control_responsive_during_bulk_and_queue_handoffs() {run(Profile::DODGY,29,false).await;}
#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn blackhole_recovers_original_intents_and_bulk_without_duplicate_effects() {run(Profile::DODGY,91,true).await;}
