use super::*;
#[tokio::test]
async fn lost_generic_response_is_replayed_but_interrupted_effect_is_never_reexecuted() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("db");let state=StateStore::load(path.clone()).await.unwrap();
    let request=ClientRequest {id:"mutation".into(),command:ClientCommand::RenameSession {session_id:"chat".into(),title:"title".into()}};
    assert!(state.reserve_operation(&request).await.unwrap().is_none());
    let response=ServerMessage::success(request.id.clone(),Some("chat".into()),None);
    state.finish_operation(request.id.clone(),&response).await.unwrap();
    let mut interrupted=request.clone();interrupted.id="interrupted".into();assert!(state.reserve_operation(&interrupted).await.unwrap().is_none());
    drop(state);
    let state=StateStore::load(path).await.unwrap();state.recover_operations().await.unwrap();
    assert!(matches!(state.reserve_operation(&request).await.unwrap(),Some(ServerMessage::Response {ok:true,uncertain:false,..})));
    assert!(matches!(state.reserve_operation(&interrupted).await.unwrap(),Some(ServerMessage::Response {ok:false,uncertain:true,..})));
    let mut conflict=request;conflict.command=ClientCommand::DeleteSession {session_id:"chat".into()};assert!(state.reserve_operation(&conflict).await.is_err());
    assert!(state.operation_receipt("mutation").await.unwrap().unwrap().complete);
}
