use super::*;
#[test]
fn dormant_replica_collection_skips_live_handles_and_authored_state() {
    let root=tempfile::tempdir().unwrap();let store=crate::store::Store::open(root.path().into()).unwrap();store.put("pair","owned",&"retained").unwrap();
    let first=store.block_cache("first").unwrap();
    for n in 0..12 {drop(store.block_cache(&format!("pair-{n}")).unwrap());}
    let databases=files(&root.path().join("blocks")).unwrap().into_iter().filter(|(p,_,_)|p.extension().is_some_and(|e|e=="sqlite3")).count();assert_eq!(databases,4);
    assert!(!first.lineage().unwrap().is_empty());assert_eq!(store.get::<String>("pair","owned").unwrap(),"retained");
    let mut live=vec![first];for n in 0..3 {live.push(store.block_cache(&format!("live-{n}")).unwrap());}
    assert!(store.block_cache("too-many-live").is_err());drop(live);assert!(store.block_cache("after-close").is_ok());
}
