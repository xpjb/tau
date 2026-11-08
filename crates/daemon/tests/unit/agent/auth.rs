use super::*;
#[tokio::test]
async fn side_by_side_auth_reads_primary_rotations_without_copying_credentials() {
    let root=tempfile::tempdir().unwrap(); let primary=root.path().join("primary.json"); let own=root.path().join("beta.json");
    let record=|access:&str,expiry:u64|json!({"openai-codex":{"type":"oauth","access":access,"refresh":"never-submit-this-refresh","accountId":"fixture-account","expires":expiry}}).to_string();
    crate::settings::atomic_write(&primary,record("first",u64::MAX).as_bytes()).await.unwrap();
    let auth=AuthStore::new(own.clone(),reqwest::Client::new()).shared_codex(Some(primary.clone()));
    assert_eq!(auth.authorization("openai-codex",None,None).await.unwrap().0,"first");
    crate::settings::atomic_write(&primary,record("rotated",u64::MAX).as_bytes()).await.unwrap();
    assert_eq!(auth.authorization("openai-codex",None,Some("first")).await.unwrap().0,"rotated");
    crate::settings::atomic_write(&primary,record("expired",0).as_bytes()).await.unwrap();
    assert_eq!(tokio::fs::read_to_string(&primary).await.unwrap(),record("expired",0));
    assert!(!own.exists(),"Reading a shared account must not copy its rotating refresh credential");
    crate::settings::atomic_write(&own,record("independent-beta",u64::MAX).as_bytes()).await.unwrap();
    assert_eq!(auth.authorization("openai-codex",None,None).await.unwrap().0,"independent-beta");
}
