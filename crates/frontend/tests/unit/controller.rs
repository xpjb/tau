use super::*;
use std::sync::Arc;
use crate::net::Event as NetworkEvent;
#[tokio::test]
async fn connection_status_does_not_popup_but_real_content_and_storage_failures_still_do() {
let root=tempfile::tempdir().unwrap();
let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
c.report_error(net::ConnectionUnavailable.into());
assert!(c.notice.is_none());assert!(c.transport_error.is_some());
c.not_sent("background-read","Connection changed",true).unwrap();assert!(c.notice.is_none());
let timeout=tokio::time::timeout(std::time::Duration::ZERO,std::future::pending::<()>()).await.unwrap_err();
c.report_sync_error(anyhow::Error::from(timeout).context("Content sync"));
assert!(c.notice.is_none());assert!(c.diagnostics().contains("Content sync"));
let error=c.download("unavailable-chat","unavailable-file",1024).unwrap_err();
c.report_error(error);assert!(c.notice.is_none());assert!(c.transport_error.as_deref().unwrap().contains("not authorized"));
c.report_sync_error(anyhow::anyhow!("Unknown block").context("Content sync"));
assert_eq!(c.notice.as_deref(),Some("Content sync: Unknown block"));
c.notice=None;c.report_error(anyhow::anyhow!("Local database is full"));
assert_eq!(c.notice.as_deref(),Some("Local database is full"));
c.notice=None;c.report_sync_error(anyhow::anyhow!("Block content integrity check failed"));
assert_eq!(c.notice.as_deref(),Some("Block content integrity check failed"));
c.notice=None;c.report_sync_error(anyhow::Error::from(std::io::Error::from(std::io::ErrorKind::PermissionDenied)).context("Write replica"));
assert!(c.notice.as_deref().unwrap().starts_with("Write replica:"),"A storage IO error is not connection loss");
let broken=tau_net::native::Frame {
    header:tau_net::native::Header::Data {version:1,offset:0,hash:String::new(),length:32,codec:tau_net::native::Codec::Zstd},
    data:b"not a zstd frame".to_vec(),
};
c.notice=None;c.report_sync_error(broken.decoded().unwrap_err().context("Content sync"));
assert!(c.notice.as_deref().unwrap().starts_with("Content sync: Invalid compressed block chunk:"),"Decompression's IO error must not be mistaken for a network error");
}

#[test]
fn unsent_messages_back_off_and_keep_the_original_id_through_failure_and_retry() {
let root = tempfile::tempdir().unwrap();
let mut c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
c.select("chat").unwrap(); c.draft("keep this intent".into()).unwrap(); c.send_prompt().unwrap();
let id = c.selected().unwrap().local.pending[0].request.id.clone();
c.chats.get_mut("chat").unwrap().local.pending[0].status = Delivery::Sending;
c.not_sent(&id, "writer full", true).unwrap();
assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::WaitingForConnection);
assert!(c.notice.is_none(), "routine retry lives on the saved message, not in a scary banner");
c.epoch = Some(1);
c.send_waiting("chat").unwrap(); // Backoff prevents touching the absent network.
assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::WaitingForConnection);
c.retry_after = None;
let db = rusqlite::Connection::open(root.path().join("client.sqlite3")).unwrap();
db.execute_batch("CREATE TRIGGER fail_send BEFORE INSERT ON local WHEN NEW.key='chat:chat' BEGIN SELECT RAISE(ABORT,'disk full');END").unwrap();
assert!(c.send_waiting("chat").is_err());
assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::WaitingForConnection, "a failed commit cannot wedge the message in Sending");
db.execute_batch("DROP TRIGGER fail_send").unwrap();
c.chats.get_mut("chat").unwrap().local.pending[0].status = Delivery::Preparing;
c.network_event(NetworkEvent::Prepared {epoch:1,id:id.clone(),result:Err("attachment changed".into()),retryable:false}).unwrap();
assert_eq!(c.selected().unwrap().local.pending[0].status, Delivery::Rejected);
c.epoch = None;
c.retry_pending(&id).unwrap();
drop(c);
let c = Controller::new(Store::open(root.path().into()).unwrap(), Arc::new(|| {})).unwrap();
let pending = &c.selected().unwrap().local.pending[0];
assert_eq!(pending.request.id, id);
assert_eq!(pending.text, "keep this intent");
assert_eq!(pending.status, Delivery::WaitingForConnection);
}

#[test]
fn lineage_fence_rolls_back_atomically_and_missing_source_work_stays_reachable() {
use crate::store::{LocalChat,Pending};
let root=tempfile::tempdir().unwrap();let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();c.select("missing").unwrap();
c.store.bind_source(&c.identity,"before").unwrap();c.account=c.store.get(&c.identity,"account").unwrap();
let local=LocalChat {draft:"keep me".into(),pending:vec![Pending {request:ClientRequest {id:"original".into(),command:ClientCommand::Prompt {session_id:"missing".into(),text:"possibly paid".into(), model: None, create: None }},text:"possibly paid".into(),files:vec![],status:Delivery::WaitingForConnection,started_at_ms:None,detail:None}],..Default::default()};
c.store.save_chat(&c.identity,"missing",&local).unwrap();
let db=rusqlite::Connection::open(root.path().join("client.sqlite3")).unwrap();db.execute_batch("CREATE TRIGGER fail_fence BEFORE UPDATE ON local WHEN NEW.key='account' BEGIN SELECT RAISE(ABORT,'fence full');END").unwrap();
assert!(c.network_event(NetworkEvent::Source(1,"after".into())).is_err());assert!(c.network_event(NetworkEvent::Ready { epoch: 1, at: std::time::Instant::now() }).is_err());assert!(c.epoch.is_none());
assert_eq!(c.store.load_chat(&c.identity,"missing").unwrap().pending[0].status,Delivery::WaitingForConnection);
assert_eq!(c.store.get::<crate::store::Account>(&c.identity,"account").unwrap().source_lineage.as_deref(),Some("before"));
db.execute_batch("DROP TRIGGER fail_fence").unwrap();c.network_event(NetworkEvent::Source(1,"after".into())).unwrap();
assert_eq!(c.selected().unwrap().local.pending[0].status,Delivery::Unconfirmed);
c.network_event(NetworkEvent::NotSent("original".into(), "late old-connection callback".into())).unwrap();
c.message(ServerMessage::Receipts {session_id:"missing".into(),reports:vec![OperationReceipt {
    id:"original".into(),accepted:false,complete:false,error:None,notice:None,
}]}).unwrap();
assert_eq!(c.selected().unwrap().local.pending[0].status,Delivery::Unconfirmed, "late events cannot clear the source fence");
c.message(ServerMessage::Sessions {sessions:vec![]}).unwrap();assert!(c.account.missing_chats.contains("missing"));assert_eq!(c.account.selected.as_deref(),Some("missing"));assert_eq!(c.selected().unwrap().local.draft,"keep me");
assert!(c.send_prompt().is_err());c.copy_missing_draft("missing").unwrap();assert_eq!(c.selected().unwrap().local.draft,"keep me");assert!(c.selected().unwrap().local.pending.is_empty());assert_eq!(c.chats["missing"].local.pending[0].request.id,"original");
}


#[test]
fn delayed_catalogue_pages_do_not_overwrite_newer_status_even_before_membership_arrives() {
let root=tempfile::tempdir().unwrap();let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
c.catalog=Some(Catalog {id:"walk".into(),..Default::default()});
c.message(ServerMessage::SessionState {session_id:"a".into(),revision:10,restore_review:None,status:SessionStatus::Running,detail:Some("new".into()),context_usage:None}).unwrap();
let a=Controller::creating_summary("a",GENERAL_PROJECT_ID,0);
// The first page is staged before its continuation can be sent offline.
assert!(c.message(ServerMessage::SessionPage {catalog_id:"walk".into(),revision:3,after:None,next:Some("a".into()),sessions:vec![a],states:std::collections::BTreeMap::from([("a".into(),9)])}).is_err());
c.message(ServerMessage::SessionState {session_id:"a".into(),revision:11,restore_review:None,status:SessionStatus::Idle,detail:Some("latest".into()),context_usage:None}).unwrap();
c.message(ServerMessage::SessionPage {catalog_id:"walk".into(),revision:3,after:Some("a".into()),next:None,sessions:vec![Controller::creating_summary("b",GENERAL_PROJECT_ID,0)],states:Default::default()}).unwrap();
assert_eq!(c.account.sessions[0].status,SessionStatus::Idle);assert_eq!(c.account.sessions[0].detail.as_deref(),Some("latest"));
c.message(ServerMessage::SessionState {session_id:"a".into(),revision:8,restore_review:None,status:SessionStatus::Running,detail:None,context_usage:None}).unwrap();
c.message(ServerMessage::SessionPage {catalog_id:"old-walk".into(),revision:1,after:None,next:None,sessions:vec![],states:Default::default()}).unwrap();
assert_eq!(c.account.sessions.len(),2);assert_eq!(c.account.sessions[0].status,SessionStatus::Idle);
}

#[test]
fn all_ongoing_and_recent_chats_sync_without_a_count_cap() {
let root=tempfile::tempdir().unwrap();
let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
let sessions=(0..20).map(|n| {
    let mut s=Controller::creating_summary(&format!("chat-{n}"),GENERAL_PROJECT_ID,n);
    s.status=SessionStatus::Idle;s.updated_at_ms=n;s
}).collect();
c.message(ServerMessage::Sessions {sessions}).unwrap();
for n in 0..13 {c.select(&format!("chat-{n}")).unwrap();}
assert_eq!(c.account.recent_chats.len(),13,"recent is time-based, not eight chats");
assert_eq!(c.sync_scopes().len(),13);
let read=c.account.read_at.clone();
c.plan_dirty.set(false);
for n in 14..20 {
    c.message(ServerMessage::SessionState {session_id:format!("chat-{n}"),revision:1,restore_review:None,
        status:SessionStatus::Running,context_usage:None,detail:None}).unwrap();
}
assert!(c.plan_dirty.get());assert_eq!(c.sync_scopes().len(),19,"all ongoing chats participate, including never-opened old chats");
assert_eq!(c.sync_scopes()[0],"chat-12");assert!(!c.sync_scopes().contains(&"chat-13".into()),"don't subscribe to cold archives");
c.message(ServerMessage::SessionState {session_id:"chat-14".into(),revision:2,restore_review:None,
    status:SessionStatus::Idle,context_usage:None,detail:None}).unwrap();
assert!(c.sync_scopes().contains(&"chat-14".into()),"a final body must catch up after the run settles");
c.account.prefetch_at.insert("chat-14".into(),crate::clock::now_ms().unwrap()-2*crate::store::RECENT_CHAT_MS);
assert!(!c.sync_scopes().contains(&"chat-14".into()));
c.account.sessions.iter_mut().find(|s|s.id=="chat-13").unwrap().updated_at_ms=crate::clock::now_ms().unwrap();
assert!(c.sync_scopes().contains(&"chat-13".into()),"recent cross-device activity is eligible too");
c.account.missing_chats.insert("chat-8".into());
c.account.sessions.iter_mut().find(|s|s.id=="chat-7").unwrap().starter=true;
assert_eq!(c.sync_scopes().len(),17);assert_eq!(c.account.read_at,read);
c.store.put(&c.identity,"account",&c.account).unwrap();drop(c);
let c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
assert_eq!(c.sync_scopes().len(),17);assert_eq!(c.account.read_at,read);
let mut legacy:Account=serde_json::from_str(r#"{"recent_chats":["old-client-chat"]}"#).unwrap();
legacy.age_recent_chats(100);assert_eq!(legacy.prefetch_at["old-client-chat"],100);
legacy.age_recent_chats(100+crate::store::RECENT_CHAT_MS+1);assert!(legacy.recent_chats.is_empty());
}

#[test]
fn list_resyncs_coalesce_without_starving_an_in_progress_traversal() {
let root=tempfile::tempdir().unwrap();let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
c.catalog=Some(Catalog {id:"in-progress".into(),..Default::default()});c.epoch=Some(1);
for _ in 0..100 {assert_eq!(c.request(ClientCommand::ListSessions).unwrap(),"in-progress");}
assert!(c.catalog.as_ref().unwrap().refresh);assert!(c.requests.is_empty());
c.epoch=None;c.message(ServerMessage::SessionPage {catalog_id:"in-progress".into(),revision:1,after:None,next:None,sessions:vec![],states:Default::default()}).unwrap();assert!(c.catalog.is_some());
assert!(c.message(ServerMessage::ProjectPage {catalog_id:"in-progress".into(),revision:1,after:None,next:None,projects:vec![Project::general()]}).is_err());assert!(c.catalog.is_none());
}

#[cfg(not(target_os = "android"))]
#[test]
fn offline_preview_copies_through_the_verified_replica_without_a_feed_fallback() {
    let root=tempfile::tempdir().unwrap();
    let mut c=Controller::new(Store::open(root.path().into()).unwrap(),Arc::new(||{})).unwrap();
    crate::demo::populate(&mut c).unwrap();
    assert!(c.network.is_none());
    c.copy_details("demo",vec!["event-1".into(),"event-2".into()]).unwrap();
    let copied=c.copied.take().unwrap();
    assert!(copied.contains("Thinking\nReview the existing client"));
    assert!(copied.contains("Input\ncargo nextest run --workspace"));
    assert!(c.copy.is_none());assert_eq!(c.notice.as_deref(),Some("Details copied"));
}
