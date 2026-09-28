#![cfg(unix)]
//! Real controller + control socket + native content against a gated unpaid
//! provider. A reply arriving in another chat must already be on disk on return.
use std::{sync::{Arc,atomic::{AtomicUsize,Ordering}},time::{Duration,Instant}};
use tau_frontend::{controller::Controller,store::{Settings,Store}};

async fn until(c:&mut Controller,predicate:impl Fn(&Controller)->bool) {
    let deadline=Instant::now()+Duration::from_secs(20);
    loop {
        c.poll();if predicate(c) {return;}
        assert!(Instant::now()<deadline,"Timed out: {} {:?}; selected={:?}",c.connection,c.notice,c.account.selected);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}
fn has(c:&Controller,chat:&str,text:&str)->bool {
    c.chats.get(chat).is_some_and(|c|c.feed.events.values().any(|e|e.text==text))
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn active_recent_chats_fetch_replies_without_selection_and_survive_client_restart() {
    use axum::{Router,Json,extract::State,response::IntoResponse,routing::post};
    use serde_json::{Value,json};
    use futures_util::StreamExt;
    struct Model {calls:AtomicUsize,gate:tokio::sync::Notify}
    async fn reply(State(model):State<Arc<Model>>,Json(_):Json<Value>)->impl IntoResponse {
        let n=model.calls.fetch_add(1,Ordering::SeqCst);
        assert!(n<3,"No implicit provider execution while prefetching");
        if n==0 || n==2 {model.gate.notified().await;}
        let text=["Background reply ","Foreground reply","Background reply after restart"][n];
        let first=format!("data: {}\n\n",json!({"choices":[{"index":0,"delta":{"content":text}}]}));
        let stream=futures_util::stream::once(async {Ok::<_,std::io::Error>(first)})
            .chain(futures_util::stream::once(async move {
                if n==0 {model.gate.notified().await;}
                Ok(format!("data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    json!({"choices":[{"index":0,"delta":{"content":if n==0 {"one café 😀"} else {""}}}]}),
                    json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"total_tokens":10}})))
            }));
        ([ ("content-type","text/event-stream") ],axum::body::Body::from_stream(stream))
    }
    let server=tempfile::tempdir().unwrap();let local=tempfile::tempdir().unwrap();
    let model=Arc::new(Model {calls:AtomicUsize::new(0),gate:tokio::sync::Notify::new()});
    let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let endpoint=listener.local_addr().unwrap();
    let app=Router::new().route("/{*path}",post(reply)).with_state(model.clone());
    let provider=tokio::spawn(async move {axum::serve(listener,app).await.unwrap()});
    let mut settings=tau_protocol::settings::Settings::default();
    settings.daemon.generate_titles=false;settings.daemon.idle_timeout_seconds=0;settings.agent.load_agents_files=false;
    let p=settings.providers.get_mut("openai-codex").unwrap();p.api=tau_protocol::settings::Api::ChatCompletions;
    p.base_url=format!("http://{endpoint}");p.web_search=false;
    std::fs::write(server.path().join("settings.json"),serde_json::to_vec(&settings).unwrap()).unwrap();
    std::fs::write(server.path().join("auth.json"),r#"{"openai-codex":{"type":"api_key","key":"unpaid-local-fixture"}}"#).unwrap();
    {use std::os::unix::fs::PermissionsExt;std::fs::set_permissions(server.path().join("auth.json"),std::fs::Permissions::from_mode(0o600)).unwrap();}
    let address=std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let daemon=tokio::spawn(taud::run(taud::Config {
        bind:address,transfer_bind:"127.0.0.1:0".parse().unwrap(),transfer_bind_v6:None,token:Arc::from("prefetch-fixture"),
        settings_path:server.path().join("settings.json"),import_pi_dir:None,codex_auth_source:None,cwd:server.path().into(),
        database_path:server.path().join("tau.sqlite3"),telemetry_path:server.path().join("crash.jsonl"),
        attachment_root:server.path().join("outbox"),upload_root:server.path().join("uploads"),
    }));
    let store=Store::open(local.path().into()).unwrap();
    store.put("","settings",&Settings {server_url:format!("http://{address}"),token:"prefetch-fixture".into()}).unwrap();
    let mut c=Controller::new(store,Arc::new(||{})).unwrap();
    until(&mut c,|c|c.epoch.is_some()).await;
    c.new_chat().unwrap();c.draft("hold background reply".into()).unwrap();c.send_prompt().unwrap();
    until(&mut c,|c|c.account.pending_create.is_none() && model.calls.load(Ordering::SeqCst)==1 && c.selected().unwrap().feed.synchronized).await;
    let a=c.account.selected.clone().unwrap();
    c.new_chat().unwrap();c.draft("foreground work".into()).unwrap();c.send_prompt().unwrap();
    until(&mut c,|c|c.account.pending_create.is_none() && c.account.selected.as_ref()!=Some(&a) && c.selected().is_some_and(|chat|chat.feed.events.values().any(|e|e.text=="Foreground reply"))).await;
    let b=c.account.selected.clone().unwrap();let read_a=c.account.read_at.get(&a).copied();
    model.gate.notify_one();
    until(&mut c,|c|has(c,&a,"Background reply ")).await;
    assert!(c.chats[&a].feed.events.values().any(|e|e.text=="Background reply " && e.phase==tau_protocol::EventPhase::Live),
        "the unselected chat receives live streaming bytes, not only a final snapshot");
    model.gate.notify_one();
    until(&mut c,|c|has(c,&a,"Background reply one café 😀")).await;
    assert_eq!(c.account.selected.as_ref(),Some(&b));
    assert_eq!(c.account.read_at.get(&a).copied(),read_a,"prefetch is not marking another chat read");
    // Returning to A has its actual body synchronously; no poll/round trip.
    c.select(&a).unwrap();assert!(has(&c,&a,"Background reply one café 😀"));
    c.draft("another gated turn".into()).unwrap();c.send_prompt().unwrap();
    until(&mut c,|_|model.calls.load(Ordering::SeqCst)==3).await;
    c.select(&b).unwrap();drop(c);
    let mut c=Controller::new(Store::open(local.path().into()).unwrap(),Arc::new(||{})).unwrap();
    assert_eq!(c.account.selected.as_ref(),Some(&b));
    until(&mut c,|c|c.epoch.is_some() && has(c,&a,"Background reply one café 😀")).await;
    model.gate.notify_one();
    until(&mut c,|c|has(c,&a,"Background reply after restart")).await;
    assert_eq!(c.account.selected.as_ref(),Some(&b));assert_eq!(model.calls.load(Ordering::SeqCst),3);
    drop(c);daemon.abort();let _=daemon.await;provider.abort();let _=provider.await;
    // Both recent transcripts are readable with no daemon, including the reply
    // fetched while its chat was never selected in this client incarnation.
    let mut c=Controller::new(Store::open(local.path().into()).unwrap(),Arc::new(||{})).unwrap();
    c.select(&a).unwrap();assert!(has(&c,&a,"Background reply after restart"));
    c.select(&b).unwrap();assert!(has(&c,&b,"Foreground reply"));
}
