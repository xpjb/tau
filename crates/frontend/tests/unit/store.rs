#[test]
fn old_thinking_checkpoint_loads_as_the_message_row() {
    let root=tempfile::tempdir().unwrap();let store=super::Store::open(root.path().into()).unwrap();
    let mut chat=super::LocalChat::default();chat.position=super::Position {key:Some("chat/thinking:event".into()),offset:12.,follow:false};
    store.save_chat("account","chat",&chat).unwrap();
    let loaded=store.load_chat("account","chat").unwrap();
    assert_eq!(loaded.position.key.as_deref(),Some("chat/event"));assert_eq!(loaded.position.offset,12.);
}

#[test]
fn opening_store_removes_legacy_connection_credentials() {
    let root=tempfile::tempdir().unwrap();
    {
        let store=super::Store::open(root.path().into()).unwrap();
        store.db.execute("INSERT INTO local VALUES('','settings',?1)",[r#"{"server_url":"http://vibe:8789","token":"obsolete","accessToken":"obsolete-too"}"#]).unwrap();
    }
    let store=super::Store::open(root.path().into()).unwrap();
    let raw:String=store.db.query_row("SELECT value FROM local WHERE account='' AND key='settings'",[],|row|row.get(0)).unwrap();
    assert_eq!(serde_json::from_str::<serde_json::Value>(&raw).unwrap(),serde_json::json!({"server_url":"http://vibe:8789"}));
    let settings:super::Settings=store.get("","settings").unwrap();
    assert_eq!(settings.server_url,"http://vibe:8789");
}

#[test]
fn source_lineage_adopts_authored_state_under_the_url_identity() {
    let root=tempfile::tempdir().unwrap();let store=super::Store::open(root.path().into()).unwrap();
    let old="credential-bound-account";let target=super::Settings {server_url:"http://vibe:8789".into()}.identity();let lineage="persistent-source";
    let mut account=super::Account::default();account.source_lineage=Some(lineage.into());account.selected=Some("chat".into());
    store.put(old,"account",&account).unwrap();
    let old_file_root=store.root.join("files").join(super::hash(old));let source=old_file_root.join("chat").join("attachment");
    std::fs::create_dir_all(source.parent().unwrap()).unwrap();std::fs::write(&source,b"authored").unwrap();
    let mut chat=super::LocalChat::default();chat.draft="keep this".into();chat.files.push(super::LocalFile {id:"attachment".into(),name:"note.txt".into(),size:8,path:source,hash:None});
    store.put(old,"chat:chat",&chat).unwrap();
    assert!(store.adopt_source(&target,lineage).unwrap());
    assert_eq!(store.db.query_row("SELECT count(*) FROM local WHERE account=?1",[old],|row|row.get::<_,u64>(0)).unwrap(),0);
    let migrated=store.load_chat(&target,"chat").unwrap();let expected=store.root.join("files").join(super::hash(&target)).join("chat").join("attachment");
    assert_eq!(migrated.draft,"keep this");assert_eq!(migrated.files[0].path,expected);assert_eq!(std::fs::read(expected).unwrap(),b"authored");
    assert!(!store.adopt_source(&target,lineage).unwrap());
}

#[test]
fn kernel_enospc_aborts_the_bounded_import_copy() {
    let mut input=std::io::Cursor::new(vec![5;32768]);let mut full=std::fs::OpenOptions::new().write(true).open("/dev/full").unwrap();
    let error=super::copy_bytes(&mut input,&mut full,50000).unwrap_err();assert!(format!("{error:#}").contains("No space left"));
}
