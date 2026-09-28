//! Actual native transport → daemon filesystem → worker → controller. Owned
//! loopback fixtures only; no provider, real credentials or production database.
use std::{sync::Arc, time::{Duration, Instant}};
use tau_frontend::{controller::Controller, store::{Settings, Store}};
use tau_protocol::files::*;

async fn until(c: &mut Controller, test: impl Fn(&Controller)->bool) {
    let deadline=Instant::now()+Duration::from_secs(20);
    loop {c.poll();if test(c){return;}assert!(Instant::now()<deadline,"remote files stalled: {} {:?}",c.connection,c.notice);tokio::time::sleep(Duration::from_millis(10)).await;}
}
async fn query(c: &mut Controller, path: Option<String>, operation: FileOperation) -> Arc<tau_frontend::file_client::Update> {
    let request=FileRequest {session_id:c.account.selected.clone().unwrap(),path,operation};
    let generation=c.view_files(Some(request)).unwrap();
    until(c,|c|c.file_update.as_ref().is_some_and(|u|u.generation==generation)).await;
    c.file_update.clone().unwrap()
}
#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn remote_files_stream_live_updates_search_parent_traversal_and_cancel_without_transcript_writes() {
    let root=tempfile::tempdir().unwrap();let cwd=root.path().join("work");std::fs::create_dir(&cwd).unwrap();
    std::fs::create_dir(cwd.join("src")).unwrap();
    let original=(0..2000u32).map(|i|format!("// line {i}: café {}\n",blake3::hash(&i.to_le_bytes()).to_hex())).collect::<String>();
    let path=cwd.join("src/main.rs");std::fs::write(&path,&original).unwrap();
    std::fs::write(cwd.join(".gitignore"),"ignored/\n").unwrap();std::fs::create_dir(cwd.join("ignored")).unwrap();std::fs::write(cwd.join("ignored/hidden.txt"),"explicit browsing allowed").unwrap();
    std::fs::write(root.path().join("outside.txt"),"outside cwd").unwrap();
    let tcp=std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let udp=std::net::UdpSocket::bind("127.0.0.1:0").unwrap().local_addr().unwrap();
    let settings_path=root.path().join("settings.json");
    let mut settings=tau_protocol::settings::Settings::default();settings.agent.load_agents_files=false;settings.daemon.generate_titles=false;settings.daemon.idle_timeout_seconds=0;
    std::fs::write(&settings_path,serde_json::to_vec(&settings).unwrap()).unwrap();
    let config=taud::Config {bind:tcp,transfer_bind:match udp {std::net::SocketAddr::V4(a)=>a,_=>unreachable!()},transfer_bind_v6:None,token:Arc::from("file-fixture"),settings_path,import_pi_dir:None,codex_auth_source:None,cwd:cwd.clone(),database_path:root.path().join("tau.sqlite3"),telemetry_path:root.path().join("crashes.jsonl"),attachment_root:root.path().join("outbox"),upload_root:root.path().join("uploads")};
    let daemon=tokio::spawn(taud::run(config));
    let client_root=tempfile::tempdir().unwrap();let store=Store::open(client_root.path().into()).unwrap();
    store.put("","settings",&Settings {server_url:format!("http://{tcp}"),token:"file-fixture".into()}).unwrap();
    let mut c=Controller::new(store,Arc::new(||{})).unwrap();until(&mut c,|c|c.epoch.is_some()).await;
    c.new_chat().unwrap();until(&mut c,|c|c.selected().is_some_and(|chat|chat.feed.synchronized)).await;
    let session=c.account.selected.clone().unwrap();c.draft("untouched comment draft".into()).unwrap();
    let update=query(&mut c,None,FileOperation::List {after:None}).await;
    assert!(matches!(&update.response,Ok(FileReply::Directory {path,entries,..}) if path==cwd.to_str().unwrap() && entries.iter().any(|e|e.name=="src" && e.directory)));
    let update=query(&mut c,Some(path.to_str().unwrap().into()),FileOperation::Open {revision:None}).await;
    let doc=update.document.as_ref().unwrap();assert_eq!(doc.text,original);let selection=tau_code_viewer::Selection::new(doc,40,42);
    let original_revision=doc.revision.clone();let generation=update.generation;
    std::fs::write(cwd.join("replacement"),format!("// inserted\n{original}")).unwrap();std::fs::rename(cwd.join("replacement"),&path).unwrap();
    until(&mut c,|c|c.file_update.as_ref().is_some_and(|u|u.generation==generation && u.document.as_ref().is_some_and(|d|d.revision!=original_revision))).await;
    let doc=c.file_update.as_ref().unwrap().document.as_ref().unwrap();
    assert_eq!(selection.reference(doc).unwrap(),format!("{}:42-44",path.display()));
    assert_eq!(doc.namespace,update.document.as_ref().unwrap().namespace);
    let update=query(&mut c,Some(cwd.join("../outside.txt").to_str().unwrap().into()),FileOperation::Open {revision:None}).await;
    assert_eq!(update.document.as_ref().unwrap().text,"outside cwd");
    let _=query(&mut c,None,FileOperation::Search {query:"srcmain".into()}).await;
    until(&mut c,|c|c.file_update.as_ref().is_some_and(|u|matches!(&u.response,Ok(FileReply::Search {entries,indexing:false,..}) if entries.len()==1))).await;
    let update=query(&mut c,None,FileOperation::Search {query:"hidden".into()}).await;
    assert!(matches!(&update.response,Ok(FileReply::Search {entries,..}) if entries.is_empty()));
    let update=query(&mut c,Some(cwd.join("ignored/hidden.txt").to_str().unwrap().into()),FileOperation::Open {revision:None}).await;
    assert_eq!(update.document.as_ref().unwrap().text,"explicit browsing allowed");
    // Rapid interest replacement is generation fenced, including already-queued
    // completed replies. Closing clears the mailbox and cancels refresh.
    c.view_files(Some(FileRequest {session_id:session.clone(),path:Some(path.to_str().unwrap().into()),operation:FileOperation::Open {revision:None}})).unwrap();
    let update=query(&mut c,Some(root.path().join("outside.txt").to_str().unwrap().into()),FileOperation::Open {revision:None}).await;
    assert_eq!(update.document.as_ref().unwrap().text,"outside cwd");
    c.view_files(None).unwrap();tokio::time::sleep(Duration::from_millis(1200)).await;c.poll();assert!(c.file_update.is_none());
    assert_eq!(c.selected().unwrap().local.draft,"untouched comment draft");
    assert!(c.selected().unwrap().local.pending.is_empty());
    let db=rusqlite::Connection::open(root.path().join("tau.sqlite3")).unwrap();
    assert_eq!(db.query_row("SELECT count(*) FROM events WHERE session_id=?1",[&session],|r|r.get::<_,u64>(0)).unwrap(),0);
    // Existence is checked on the daemon, not trusted from a client-supplied cwd.
    let generation=c.view_files(Some(FileRequest {session_id:"missing-chat".into(),path:None,operation:FileOperation::List {after:None}})).unwrap();
    until(&mut c,|c|c.file_update.as_ref().is_some_and(|u|u.generation==generation)).await;
    assert!(c.file_update.as_ref().unwrap().response.is_err());
    drop(c);daemon.abort();let _=daemon.await;
}
