#![cfg(unix)]
//! Real TCP/QUIC on private loopback aliases. No host qdisc, routes, services,
//! external provider or relay is touched. TCP bytes and UDP datagrams share a
//! delayed, bandwidth-limited FIFO in each direction; UDP also loses packets.
use std::{net::SocketAddr,sync::{Arc,atomic::{AtomicU64,Ordering}},time::{Duration,Instant}};
use tokio::{io::{AsyncReadExt,AsyncWriteExt},net::{TcpListener,TcpStream,UdpSocket},sync::Mutex};
use tau_frontend::{controller::Controller,store::{Settings,Store}};
use tau_protocol::*;

struct Link {next:[Mutex<tokio::time::Instant>;2],udp:AtomicU64,dropped:AtomicU64,udp_bytes:AtomicU64}
impl Link {
    fn new()->Arc<Self> {Arc::new(Self {next:[Mutex::new(tokio::time::Instant::now()),Mutex::new(tokio::time::Instant::now())],udp:AtomicU64::new(0),dropped:AtomicU64::new(0),udp_bytes:AtomicU64::new(0)})}
    async fn wait(&self,direction:usize,bytes:usize) {
        let at={let mut next=self.next[direction].lock().await;
            *next=(*next).max(tokio::time::Instant::now()+Duration::from_millis(35))+Duration::from_secs_f64(bytes as f64/(64.*1024.));*next};
        tokio::time::sleep_until(at).await;
    }
}
struct Proxy {address:SocketAddr,tasks:Vec<tokio::task::JoinHandle<()>>}
impl Drop for Proxy {fn drop(&mut self) {for task in &self.tasks {task.abort();}}}
impl Proxy {
    async fn new(alias:&str,tcp:SocketAddr,udp:SocketAddr,link:Arc<Link>)->Self {
        let listener=TcpListener::bind(format!("{alias}:0")).await.unwrap();let address=listener.local_addr().unwrap();
        let control=link.clone();
        let tcp_task=tokio::spawn(async move {
            let mut connections=tokio::task::JoinSet::new();
            loop {tokio::select! {
                incoming=listener.accept()=>{
                    let (socket,_)=incoming.unwrap();let link=control.clone();
                    connections.spawn(async move {
                        let peer=TcpStream::connect(tcp).await.unwrap();let (a,b)=socket.into_split();let(c,d)=peer.into_split();
                        async fn copy(mut from:tokio::net::tcp::OwnedReadHalf,mut to:tokio::net::tcp::OwnedWriteHalf,link:Arc<Link>,direction:usize) {
                            let mut buf=[0;4096];while let Ok(n)=from.read(&mut buf).await {if n==0 {break;}link.wait(direction,n).await;if to.write_all(&buf[..n]).await.is_err() {break;}}
                        }
                        tokio::select! {_=copy(a,d,link.clone(),0)=>{},_=copy(c,b,link,1)=>{}}
                    });
                }
                _=connections.join_next(),if !connections.is_empty()=>{}
            }}
        });
        // Same advertised port on another loopback address: the transparent TCP
        // proxy need not terminate WebSockets or accidentally answer their pings.
        let socket=Arc::new(UdpSocket::bind(format!("{alias}:{}",udp.port())).await.unwrap());
        let udp_task=tokio::spawn(async move {
            let mut client=None;let mut buf=[0;65536];let mut packets=tokio::task::JoinSet::new();
            loop {tokio::select! {
                received=socket.recv_from(&mut buf)=>{
                    let (n,from)=received.unwrap();let direction=usize::from(from==udp);
                    let target=if direction==1 {let Some(client)=client else {continue;};client} else {client=Some(from);udp};
                    let serial=link.udp.fetch_add(1,Ordering::Relaxed)+1;
                    if serial%23==0 || packets.len()>=512 {link.dropped.fetch_add(1,Ordering::Relaxed);continue;}
                    let bytes=buf[..n].to_vec();let socket=socket.clone();let link=link.clone();
                    packets.spawn(async move {link.wait(direction,n).await;if socket.send_to(&bytes,target).await.is_ok() {link.udp_bytes.fetch_add(n as u64,Ordering::Relaxed);}});
                }
                _=packets.join_next(),if !packets.is_empty()=>{}
            }}
        });
        Self {address,tasks:vec![tcp_task,udp_task]}
    }
}
async fn until(a:&mut Controller,b:&mut Controller,predicate:impl Fn(&Controller,&Controller)->bool) {
    let start=Instant::now();loop {a.poll();b.poll();if predicate(a,b) {return;}
        assert!(start.elapsed()<Duration::from_secs(75),"Impaired link stalled: A={} {:?}; B={} {:?}",a.connection,a.notice,b.connection,b.notice);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test(flavor="multi_thread",worker_threads=4)]
async fn native_loss_delay_bandwidth_upload_cancel_resume_and_two_client_control() {
    use axum::{Router,Json,routing::post,extract::State,response::IntoResponse};
    use serde_json::{json,Value};
    struct Model {calls:AtomicU64,root:std::path::PathBuf,code:String}
    async fn reply(State(model):State<Arc<Model>>,Json(_):Json<Value>)->impl IntoResponse {
        let n=model.calls.fetch_add(1,Ordering::Relaxed);assert!(n<2,"Unexpected provider replay");
        let delta=if n==0 {let mut calls=vec![
            json!({"index":0,"id":"visible-file","type":"function","function":{"name":"send_file","arguments":json!({"path":model.root.join("outbox/visible.bin")}).to_string()}}),
            json!({"index":1,"id":"unread-file","type":"function","function":{"name":"send_file","arguments":json!({"path":model.root.join("outbox/unread.bin")}).to_string()}})
        ];calls.extend((0..24).map(|n|json!({"index":n+2,"id":format!("read-{n}"),"type":"function","function":{"name":"read","arguments":json!({"path":model.root.join("read.txt")}).to_string()}})));json!({"tool_calls":calls})} else {json!({"content":model.code})};
        let body=format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",json!({"choices":[{"index":0,"delta":delta}]}),json!({"choices":[{"index":0,"delta":{},"finish_reason":if n==0 {"tool_calls"} else {"stop"}}],"usage":{"total_tokens":100}}));
        ([("content-type","text/event-stream")],body)
    }
    let source=tempfile::tempdir().unwrap();std::fs::create_dir(source.path().join("outbox")).unwrap();
    std::fs::write(source.path().join("read.txt"),"READ-FIXTURE ".repeat(160)).unwrap();
    let mut bytes=vec![0;512*1024];blake3::Hasher::new().update(b"native impaired link fixture").finalize_xof().fill(&mut bytes);
    for name in ["visible.bin","unread.bin"] {std::fs::write(source.path().join("outbox").join(name),&bytes).unwrap();}
    let code=format!("```rust\n{}\n```",(0..500u32).map(|n|format!("let value_{n} = \"{}\";\n",blake3::hash(&n.to_le_bytes()).to_hex())).collect::<String>());
    assert!(code.len()>40*1024);
    let model=Arc::new(Model {calls:AtomicU64::new(0),root:source.path().into(),code:code.clone()});
    let listener=TcpListener::bind("127.0.0.1:0").await.unwrap();let model_address=listener.local_addr().unwrap();
    let app=Router::new().route("/{*path}",post(reply)).with_state(model.clone());let provider=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let mut settings=tau_protocol::settings::Settings::default();settings.agent.load_agents_files=false;settings.daemon.generate_titles=false;settings.daemon.idle_timeout_seconds=0;
    let p=settings.providers.get_mut("openai-codex").unwrap();p.api=tau_protocol::settings::Api::ChatCompletions;p.base_url=format!("http://{model_address}");p.web_search=false;
    std::fs::write(source.path().join("settings.json"),serde_json::to_vec(&settings).unwrap()).unwrap();
    std::fs::write(source.path().join("auth.json"),r#"{"openai-codex":{"type":"api_key","key":"local-fixture"}}"#).unwrap();
    {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(source.path().join("auth.json"),std::fs::Permissions::from_mode(0o600)).unwrap();}
    let tcp=std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();let udp=std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let config=taud::Config {bind:tcp,transfer_bind:match udp {SocketAddr::V4(a)=>a,_=>unreachable!()},token:Arc::from("native-link-fixture"),settings_path:source.path().join("settings.json"),import_pi_dir:None,codex_auth_source:None,cwd:source.path().into(),database_path:source.path().join("tau.sqlite3"),telemetry_path:source.path().join("crashes.jsonl"),attachment_root:source.path().join("outbox"),upload_root:source.path().join("uploads")};
    let daemon=tokio::spawn(taud::run(config));
    // Wait for the listener before starting transparent proxies.
    for _ in 0..200 {if TcpStream::connect(tcp).await.is_ok() {break;}tokio::time::sleep(Duration::from_millis(10)).await;}
    let link=Link::new();let pa=Proxy::new("127.0.0.2",tcp,udp,link.clone()).await;let pb=Proxy::new("127.0.0.3",tcp,udp,link.clone()).await;
    let ar=tempfile::tempdir().unwrap();let br=tempfile::tempdir().unwrap();
    let connect=|root:&std::path::Path,address:SocketAddr| {let store=Store::open(root.into()).unwrap();store.put("","settings",&Settings {server_url:format!("http://{address}"),token:"native-link-fixture".into()}).unwrap();Controller::new(store,Arc::new(||{})).unwrap()};
    let mut a=connect(ar.path(),pa.address);let mut b=connect(br.path(),pb.address);
    until(&mut a,&mut b,|a,b|a.epoch.is_some()&&b.epoch.is_some()).await;
    a.new_chat().unwrap();until(&mut a,&mut b,|a,_|a.selected().is_some_and(|c|c.feed.synchronized)).await;
    let session=a.account.selected.clone().unwrap();until(&mut a,&mut b,|_,b|b.account.sessions.iter().any(|s|s.id==session)).await;b.select(&session).unwrap();
    let attachment=ar.path().join("input.bin");std::fs::write(&attachment,&bytes[..256*1024]).unwrap();a.attach(&attachment,None).unwrap();a.draft("Inspect this owned upload and send both fixture files".into()).unwrap();a.send_prompt().unwrap();
    until(&mut a,&mut b,|a,b|[a,b].iter().all(|c|c.selected().is_some_and(|chat|chat.feed.events.values().any(|e|e.text==code)))).await;
    {
        let chat=a.chats.get_mut(&session).unwrap();chat.local.details_default=true;
        for event in chat.feed.events.values().filter(|e|e.kind==tau_protocol::EventKind::Tool) {
            if let Some(call)=&event.tool_call_id {chat.local.expansion.insert(format!("tool:{call}"),true);chat.local.expansion.insert(format!("tool:{call}:Output"),true);chat.local.expansion.insert(format!("tool:{call}:Input"),true);}
        }
    }
    a.save_chat(&session).unwrap();
    until(&mut a,&mut b,|a,_|a.selected().unwrap().feed.events.values().filter(|e|e.role==tau_protocol::EventRole::Tool && e.tool_name.as_deref()==Some("read") && e.text.contains("READ-FIXTURE")).count()==24).await;
    assert!(!b.selected().unwrap().feed.events.values().any(|e|e.text.contains("READ-FIXTURE")),"Collapsed client fetched hidden tool bodies");
    until(&mut a,&mut b,|a,b|[a,b].iter().all(|c|c.selected().unwrap().feed.block_states.values().all(|s|s=="completed"))).await;
    let entries=a.selected().unwrap().feed.events.values().filter_map(|e|e.attachment.as_ref().map(|f|(f.file_name.clone(),e.entry_id.clone()))).collect::<std::collections::HashMap<_,_>>();
    assert!(entries.contains_key("visible.bin")&&entries.contains_key("unread.bin"),"Missing attachment cards: {entries:?}");
    let visible=&entries["visible.bin"];let unread=&entries["unread.bin"];
    let download_started=Instant::now();
    let key=Controller::download_key(&session,visible);let path=a.download(&session,visible,MAX_UPLOAD_BYTES as u64).unwrap();
    until(&mut a,&mut b,|a,_|a.downloads[&key].status.transferred>=64*1024).await;
    let started=Instant::now();let id=b.request(ClientCommand::RenameSession {session_id:session.clone(),title:"Control during saturated native data".into()}).unwrap();
    until(&mut a,&mut b,|_,b|!b.account.pending_controls.contains_key(&id)).await;
    let acceptance=started.elapsed();assert!(acceptance<Duration::from_secs(4),"Control receipt starved behind native bulk: {acceptance:?}");
    a.cancel_download(&key).unwrap();until(&mut a,&mut b,|a,_|a.downloads[&key].status.done).await;assert!(a.downloads[&key].status.failure.is_some());assert!(!path.exists());
    a.download(&session,visible,MAX_UPLOAD_BYTES as u64).unwrap();until(&mut a,&mut b,|a,_|a.downloads[&key].status.done).await;assert!(a.downloads[&key].status.failure.is_none(),"{:?}",a.downloads[&key].status.failure);
    assert_eq!(std::fs::read(path).unwrap(),bytes);assert!(download_started.elapsed()>=Duration::from_secs(7),"Native file bypassed the 64 KiB/s shaper");
    until(&mut a,&mut b,|a,b|a.native_metrics.resumed_bytes>=64*1024&&a.native_metrics.content_rx_bytes>=512*1024&&b.health.min_max().is_some()).await;
    assert_eq!(a.native_metrics.connections,1);assert_eq!(b.native_metrics.connections,1);
    assert_eq!(a.native_metrics.integrity_failures,0);assert!(a.native_metrics.cancelled_streams>0);assert!(link.dropped.load(Ordering::Relaxed)>0);
    assert!(link.udp_bytes.load(Ordering::Relaxed)>768*1024,"QUIC bypassed the packet shaper");
    assert!(a.health.min_max().unwrap().1<Duration::from_secs(5));assert!(b.health.min_max().unwrap().1<Duration::from_secs(5));
    let db=rusqlite::Connection::open(source.path().join("tau.sqlite3")).unwrap();
    let unread_parts:u64=db.query_row("SELECT count(*) FROM block_parts WHERE scope=?1 AND id=?2",rusqlite::params![session,format!("file:{unread}")],|r|r.get(0)).unwrap();assert_eq!(unread_parts,0,"Unread file bytes were prefetched");
    assert_eq!(model.calls.load(Ordering::Relaxed),2);
    eprintln!("native-link: receipt={acceptance:?}, dropped={}, shaped UDP={} bytes\nA: {}\nB: {}",link.dropped.load(Ordering::Relaxed),link.udp_bytes.load(Ordering::Relaxed),a.diagnostics(),b.diagnostics());
    drop(a);drop(b);drop(pa);drop(pb);daemon.abort();provider.abort();let _=daemon.await;let _=provider.await;
}
