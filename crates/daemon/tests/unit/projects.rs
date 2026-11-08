use super::*;
#[tokio::test]
async fn version_one_database_migrates_in_place_without_changing_history_or_prompt() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("old.sqlite3");
    let db = rusqlite::Connection::open(&path).unwrap();
    db.execute_batch(include_str!("../../src/schema.sql")).unwrap();
    let data = serde_json::json!({"title":"Existing","starter":true,"parent_id":null,"model":{"provider":"openai-codex","modelId":"gpt-6-astra"},
        "thinking":"high","created_at_ms":12,"updated_at_ms":20,"tokens":null,"needs_turn":false,"head":"entry","next_order":0,"revision":3}).to_string();
    db.execute("INSERT INTO sessions(id,starter,activity,data,queue) VALUES('old',1,20,?1,?2)",params![data,serde_json::to_string(&tau_net::QueueState::native()).unwrap()]).unwrap();
    db.execute("INSERT INTO entries(session_id,id,kind,data) VALUES('old','entry','model_change',?1)",
        [r#"{"id":"entry","type":"model_change","provider":"openai-codex","modelId":"gpt-6-astra","thinkingLevel":"high"}"#]).unwrap();
    drop(db);
    let state = StateStore::load(path.clone()).await.unwrap();
    let old = state.get("old").await.unwrap().unwrap();
    assert_eq!((old.project_id.as_str(),old.project_prompt.as_str()),("general",""));
    assert_eq!((old.revision,old.updated_at_ms,old.head.as_deref()),(3,20,Some("entry")));
    assert_eq!(state.projects().await.unwrap(),vec![Project::general()]);
    let project = uuid::Uuid::new_v4().to_string();
    state.create_project(project.clone(),"Build".into(),"Pinned".into()).await.unwrap();
    let chat = state.create(old.model.clone(),old.thinking.clone(),None,project.clone()).await.unwrap();
    assert_ne!(chat,"old");
    assert_eq!(state.create(old.model,old.thinking,None,"general".into()).await.unwrap(),"old");
    state.update_project(project.clone(),0,"Build".into(),"Edited".into()).await.unwrap();
    assert_eq!(state.project_prompt(&chat).await.unwrap(),"Pinned");
    state.move_session(chat.clone(),project).await.unwrap();
    assert_eq!(state.project_prompt(&chat).await.unwrap(),"Pinned","Moving to the current project is not an apply-latest backdoor");
    let again = StateStore::load(path).await.unwrap();
    assert_eq!(again.project_prompt(&chat).await.unwrap(),"Pinned");
    again.access(|db| {
        assert_eq!(db.query_row("PRAGMA user_version",[],|r|r.get::<_,u32>(0))?,5);
        assert_eq!(db.query_row("SELECT count(*) FROM entries WHERE session_id='old'",[],|r|r.get::<_,u32>(0))?,1);
        Ok(())
    }).await.unwrap();
}
