use super::*;
#[tokio::test]
async fn outbox_original_edits_cannot_change_an_old_card_and_owned_corruption_is_rejected() {
    let root=tempfile::tempdir().unwrap();let original=root.path().join("pixel.png");let bytes=b"\x89PNG\r\n\x1a\nfixture";
    fs::write(&original,bytes).await.unwrap();let result=send_file(root.path(),&original,None,&CancellationToken::new()).await.unwrap();
    let entry=json!({"type":"message","message":{"role":"toolResult","details":result["details"]}});let request=attachment_request(&entry).unwrap();assert_ne!(request.path,original);assert!(request.sha256.is_some());
    fs::write(&original,b"edited original").await.unwrap();assert_eq!(fs::read(&request.path).await.unwrap(),bytes);image_reference(root.path(),&request).await.unwrap();
    let mut changed=bytes.to_vec();*changed.last_mut().unwrap()^=1;fs::write(&request.path,changed).await.unwrap();assert!(image_reference(root.path(),&request).await.unwrap_err().to_string().contains("integrity"));
}
