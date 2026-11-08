use super::*;
#[test]
fn duplicate_updates_do_not_extend_popups_but_new_alerts_get_their_own_deadline() {
    let now = Instant::now(); let mut popup = NoticePopup::default();
    assert!(popup.observe(Some(&Notice::from("Saved")), now));
    assert!(!popup.observe(Some(&Notice::from("Saved")), now + Duration::from_secs(3)));
    assert_eq!(popup.remaining(now + Duration::from_secs(3)), Some(Duration::from_secs(1)));
    assert!(popup.observe(Some(&Notice::from("Saved")), now + LIFETIME)); assert!(!popup.visible());
    assert!(!popup.observe(Some(&Notice::from("Saved")), now + LIFETIME * 2));
    assert!(popup.observe(Some(&Notice::from("File saved")), now + LIFETIME * 2)); assert!(popup.visible());
    assert!(popup.observe(None, now + LIFETIME * 2)); assert!(!popup.visible());
    assert!(popup.observe(Some(&Notice::from("File saved")), now + LIFETIME * 2)); assert!(popup.visible());
}
#[test]
fn equal_notice_text_with_different_destinations_gets_a_new_deadline() {
    use crate::notice::DownloadTarget;
    let now = Instant::now();
    let mut popup = NoticePopup::default();
    let target = DownloadTarget { identity: "account".into(), lineage: "source".into(), session: "chat".into(), entry: "first".into() };
    let first = Notice::download("Saved to Downloads/Tau".into(), target.clone());
    let second = Notice::download(first.to_string(), DownloadTarget { entry: "second".into(), ..target });
    assert!(popup.observe(Some(&first), now));
    assert!(!popup.observe(Some(&first), now + Duration::from_secs(3)));
    assert!(popup.observe(Some(&second), now + Duration::from_secs(3)));
    assert_eq!(popup.remaining(now + Duration::from_secs(3)), Some(LIFETIME));
    let ordinary = Notice::from(second.to_string());
    assert!(popup.observe(Some(&ordinary), now + Duration::from_secs(4)));
    assert!(popup.message.as_ref().unwrap().download.is_none());
}
