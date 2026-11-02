#[test]
fn old_thinking_checkpoint_loads_as_the_message_row() {
    let root=tempfile::tempdir().unwrap();let store=super::Store::open(root.path().into()).unwrap();
    let mut chat=super::LocalChat::default();chat.position=super::Position {key:Some("chat/thinking:event".into()),offset:12.,follow:false};
    store.save_chat("account","chat",&chat).unwrap();
    let loaded=store.load_chat("account","chat").unwrap();
    assert_eq!(loaded.position.key.as_deref(),Some("chat/event"));assert_eq!(loaded.position.offset,12.);
}

#[test]
fn kernel_enospc_aborts_the_bounded_import_copy() {
    let mut input=std::io::Cursor::new(vec![5;32768]);let mut full=std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
    let error=super::copy_bytes(&mut input,&mut full,50000).unwrap_err();assert!(format!("{error:#}").contains("No space left"));
}
