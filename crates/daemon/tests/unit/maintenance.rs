use super::*;
#[tokio::test]
async fn restore_rotation_is_exclusive_and_keeps_mutation_ownership() {
    let root=tempfile::tempdir().unwrap();let path=root.path().join("db");
    let state=crate::state::StateStore::load(path.clone()).await.unwrap();let old=state.block_cursor().await.unwrap();
    let request=tau_net::ClientRequest {id:"reserved".into(),command:tau_net::ClientCommand::RenameSession {session_id:"s".into(),title:"new".into()}};
    state.reserve_operation(&request).await.unwrap();drop(state);
    let lease=DatabaseLease::acquire(&path).unwrap();assert!(rotate_lineage(&path).await.is_err());drop(lease);
    let new=rotate_lineage(&path).await.unwrap();assert_ne!(new,old.lineage);
    let state=crate::state::StateStore::load(path).await.unwrap();
    assert!(matches!(state.reserve_operation(&request).await.unwrap(),Some(tau_net::ServerMessage::Response {uncertain:true,..})));
}
