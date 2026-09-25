use super::*;
use tau_blocks::{UploadSpec,UploadPurpose,BLOCK_CHUNK_BYTES};

fn file_spec(id:&str,session:&str,bytes:&[u8])->UploadSpec {UploadSpec {id:id.into(),length:bytes.len() as u64,hash:blake3::hash(bytes).to_hex().to_string(),purpose:UploadPurpose::File {session_id:session.into(),file_name:"kept.bin".into()}}}
async fn upload(manager:&AgentManager,spec:&UploadSpec,bytes:&[u8]) {
    manager.begin_upload(spec.clone()).await.unwrap();
    for (n,chunk) in bytes.chunks(BLOCK_CHUNK_BYTES).enumerate() {manager.write_upload(spec.clone(),(n*BLOCK_CHUNK_BYTES) as u64,chunk.to_vec()).await.unwrap();}
}

#[tokio::test]
async fn sqlite_full_cannot_accept_a_prompt_or_start_provider_work() {
    let mut model=ModelServer::start(vec![completion("done",vec![])]).await;
    let (_root,manager,_,server)=fixture(&model,Api::ChatCompletions).await;
    let id=manager.create_session(None,"general").await.unwrap();
    let before=manager.inner.state.block_cursor().await.unwrap();
    manager.inner.state.access(|db| {let pages:u64=db.query_row("PRAGMA page_count",[],|r|r.get(0))?;db.pragma_update(None,"max_page_count",pages)?;Ok(())}).await.unwrap();
    let text="full database ".repeat(4000);
    let error=manager.prompt(&id,&text,"full-prompt").await.err().unwrap();assert!(format!("{error:#}").contains("full"));
    assert!(manager.inner.state.receipt(&id,"full-prompt").await.unwrap().is_none());assert!(manager.inner.state.queue(&id).await.unwrap().requests.is_empty());
    assert_eq!(manager.inner.state.block_cursor().await.unwrap(),before);assert!(model.requests.try_recv().is_err());
    manager.inner.state.access(|db| {db.pragma_update(None,"max_page_count",8192)?;Ok(())}).await.unwrap();
    manager.prompt(&id,&text,"full-prompt").await.unwrap();model.request().await;
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn upload_ownership_survives_fork_parent_delete_and_rejects_changed_publications() {
    let model=ModelServer::start(vec![]).await;let (_root,manager,_,server)=fixture(&model,Api::ChatCompletions).await;
    let id=manager.create_session(None,"general").await.unwrap();let bytes=vec![42;BLOCK_CHUNK_BYTES*3];let spec=file_spec("owned",&id,&bytes);upload(&manager,&spec,&bytes).await;
    let (a,b)=tokio::join!(manager.finish_upload(spec.clone()),manager.finish_upload(spec.clone()));let path=a.unwrap().file.unwrap().path;assert_eq!(path,b.unwrap().file.unwrap().path);
    let child=manager.clone_session(&id).await.unwrap();let grandchild=manager.clone_session(&child).await.unwrap();
    manager.delete_session(&id).await.unwrap();manager.delete_session(&child).await.unwrap();assert_eq!(std::fs::read(&path).unwrap(),bytes);
    // The retained export cannot be rebound after its pure upload lease expires.
    manager.inner.state.access(|db| {db.execute("DELETE FROM blocks WHERE scope='@uploads'",[])?;Ok(())}).await.unwrap();
    let other=manager.create_session(None,"general").await.unwrap();let changed=file_spec("owned",&other,b"different");upload(&manager,&changed,b"different").await;
    assert!(manager.finish_upload(changed).await.unwrap_err().to_string().contains("bound"));assert_eq!(std::fs::read(&path).unwrap(),bytes);
    manager.delete_session(&grandchild).await.unwrap();assert!(!std::path::Path::new(&path).exists());
    let late=file_spec("late",&other,b"abcd");manager.begin_upload(late.clone()).await.unwrap();manager.delete_session(&other).await.unwrap();assert!(manager.write_upload(late.clone(),0,b"abcd".to_vec()).await.is_err());assert!(manager.finish_upload(late).await.is_err());
    manager.shutdown().await;server.abort();
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
async fn cancelling_after_file_rename_keeps_finalization_owned_until_sqlite_seals() {
    let model=ModelServer::start(vec![]).await;let (_root,manager,_,server)=fixture(&model,Api::ChatCompletions).await;
    let id=manager.create_session(None,"general").await.unwrap();let bytes=vec![13;BLOCK_CHUNK_BYTES*8];let spec=file_spec("cancel-publication",&id,&bytes);upload(&manager,&spec,&bytes).await;
    let publication=manager.inner.upload_publication.lock().await;
    let task=tokio::spawn({let manager=manager.clone();let spec=spec.clone();async move {manager.finish_upload(spec).await}});
    let dir=manager.inner.config.upload_root.join(&id);let end=std::time::Instant::now()+Duration::from_secs(5);
    loop {
        if std::fs::read_dir(&dir).is_ok_and(|entries|entries.flatten().any(|e|e.metadata().is_ok_and(|m|m.len()==bytes.len() as u64))) {break;}
        assert!(std::time::Instant::now()<end);tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let (entered,waiting)=tokio::sync::oneshot::channel();let (release,blocked)=std::sync::mpsc::channel();
    let gate=tokio::spawn({let state=manager.inner.state.clone();async move {state.access(move |_| {let _=entered.send(());blocked.recv()?;Ok(())}).await}});waiting.await.unwrap();drop(publication);
    let target=dir.join(format!("{}-kept.bin",blake3::hash(&serde_json::to_vec(&spec).unwrap()).to_hex()));
    while !target.exists() {assert!(std::time::Instant::now()<end);tokio::time::sleep(Duration::from_millis(5)).await;}
    task.abort();assert!(task.await.unwrap_err().is_cancelled());
    assert!(manager.inner.block_imports.clone().try_acquire_many_owned(2).is_err(),"Cancelled caller released a still-running finalizer's capacity");
    release.send(()).unwrap();gate.await.unwrap().unwrap();
    loop {if manager.begin_upload(spec.clone()).await.unwrap().sealed {break;}assert!(std::time::Instant::now()<end);tokio::time::sleep(Duration::from_millis(5)).await;}
    assert_eq!(std::fs::read(&target).unwrap(),bytes);std::fs::write(&target,vec![14;bytes.len()]).unwrap();assert!(manager.begin_upload(spec).await.is_err());
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn catalogue_pages_reset_on_membership_change_and_delete_many_cold_chats() {
    let model=ModelServer::start(vec![]).await;let (_root,manager,_,server)=fixture(&model,Api::ChatCompletions).await;
    let topic=uuid::Uuid::new_v4().to_string();manager.create_project(topic.clone(),"Many".into(),"context".repeat(9000)).await.unwrap();
    let first=manager.create_session(None,&topic).await.unwrap();manager.close_session(&first).await.unwrap();
    manager.inner.state.access({let first=first.clone();move |db| {
        let raw:String=db.query_row("SELECT data FROM sessions WHERE id=?1",[first],|r|r.get(0))?;let tx=db.transaction()?;
        for n in 0..140 {let id=format!("cold-{n:03}");tx.execute("INSERT INTO sessions(id,data,queue,activity,starter) VALUES(?1,?2,?3,0,0)",rusqlite::params![id,raw,serde_json::to_string(&tau_protocol::QueueState::native())?])?;}tx.commit()?;Ok(())
    }}).await.unwrap();
    let count=manager.inner.runtimes.lock().await.len();
    let tau_protocol::ServerMessage::SessionPage {revision,next,sessions,..}=manager.list_page("catalog".into(),false,None,0).await.unwrap() else {panic!()};assert_eq!(sessions.len(),64);assert!(next.is_some());
    let tau_protocol::ServerMessage::SessionPage {sessions:second,after,..}=manager.list_page("catalog".into(),false,next.clone(),revision).await.unwrap() else {panic!()};assert_eq!(after,next);assert_eq!(second.len(),64);assert!(sessions.iter().all(|a|second.iter().all(|b|a.id!=b.id)));
    assert_eq!(manager.inner.runtimes.lock().await.len(),count);
    manager.inner.state.rename(&first,"Changed".into(),false).await.unwrap();
    let tau_protocol::ServerMessage::SessionPage {revision:new,after,..}=manager.list_page("catalog".into(),false,next,revision).await.unwrap() else {panic!()};assert_ne!(new,revision);assert!(after.is_none());
    for n in 0..10 {manager.create_project(uuid::Uuid::new_v4().to_string(),format!("Other {n}"),String::new()).await.unwrap();}
    let tau_protocol::ServerMessage::ProjectPage {projects,next,..}=manager.list_page("catalog".into(),true,None,0).await.unwrap() else {panic!()};assert_eq!(projects.len(),8);assert!(next.is_some());
    manager.delete_project(topic,0,tau_protocol::DeleteProjectMode::DeleteChats).await.unwrap();assert!(manager.inner.state.list().await.unwrap().is_empty());
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn restored_execution_requires_review_and_review_does_not_run_or_replay() {
    let model=ModelServer::start(vec![]).await;let (root,manager,_,server)=fixture(&model,Api::ChatCompletions).await;
    let id=manager.create_session(None,"general").await.unwrap();
    let config=manager.inner.config.clone();manager.shutdown().await;server.abort();drop(manager);
    crate::maintenance::rotate_lineage(&root.path().join("tau.sqlite3")).await.unwrap();
    let manager=AgentManager::new(config,StateStore::load(root.path().join("tau.sqlite3")).await.unwrap()).await.unwrap();
    assert!(manager.prompt(&id,"not yet","guarded").await.err().unwrap().to_string().contains("Restored"));
    let child=manager.clone_session(&id).await.unwrap();assert!(manager.inner.state.restore_review(&child).await.unwrap());
    let generation=format!("{}:{id}",manager.inner.state.block_cursor().await.unwrap().lineage);
    assert!(manager.queue_control(&id,&generation,"resume-restored",tau_protocol::QueueOperation::Resume {run_id:None}).await.is_err());
    manager.inner.state.review_restore(&id).await.unwrap();assert!(!manager.inner.state.restore_review(&id).await.unwrap());
    assert!(manager.inner.state.receipt(&id,"guarded").await.unwrap().is_none());assert!(manager.inner.state.queue(&id).await.unwrap().requests.is_empty());
    assert_eq!(manager.runtime(&id).await.unwrap().snapshot().status,tau_protocol::SessionStatus::Idle);
    manager.shutdown().await;server.abort();
}

#[tokio::test]
async fn schema_four_migration_adopts_transitive_fork_owners_and_streaming_export_keeps_source_safe() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("source.sqlite3");let state=StateStore::load(path.clone()).await.unwrap();
    let parent=state.create(Settings::default().agent.model,"medium".into(),None,"general".into()).await.unwrap();let (child,_)=state.branch(&parent,None).await.unwrap();let (grandchild,_)=state.branch(&child,None).await.unwrap();
    let spec=file_spec("legacy-file",&parent,b"old");
    state.access(move |db| {
        let tx=db.transaction()?;tau_blocks::uploads::begin(&tx,&spec)?;tau_blocks::uploads::write(&tx,&spec,0,b"old")?;
        tau_blocks::uploads::seal(&tx,&spec,&spec.hash,Some(tau_protocol::UploadedFile {name:"old".into(),path:"/private/legacy/old".into(),size:3}))?;tx.commit()?;
        db.execute_batch("DROP TABLE file_owners; DROP TABLE file_publications; DROP TABLE restore_guards;
          DROP TRIGGER catalogue_insert; DROP TRIGGER catalogue_delete; DROP TRIGGER catalogue_session;
          DROP TRIGGER catalogue_project_insert; DROP TRIGGER catalogue_project_update; DROP TRIGGER catalogue_project_delete;
          DROP TRIGGER session_admission; DROP TABLE catalogue_clock; PRAGMA user_version=4;")?;Ok(())
    }).await.unwrap();drop(state);
    let state=StateStore::load(path.clone()).await.unwrap();let owners=state.access(|db|Ok(db.prepare("SELECT session FROM file_owners ORDER BY session")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?)).await.unwrap();
    assert_eq!(owners.len(),3);for id in [&parent,&child,&grandchild] {assert!(owners.contains(id));}
    assert!(state.export_history(&parent,&path).await.is_err());
    let export=root.path().join("history.json");state.export_history(&parent,&export).await.unwrap();let value:Value=serde_json::from_slice(&std::fs::read(export).unwrap()).unwrap();assert_eq!(value["sessionId"],parent);assert_eq!(value["format"],"tau-history");
    state.access(|db| {db.execute_batch("PRAGMA user_version=999")?;Ok(())}).await.unwrap();assert!(StateStore::load(path).await.is_err());assert!(state.get(&parent).await.unwrap().is_some());
}
