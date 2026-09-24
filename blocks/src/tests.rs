use super::*;
use serde_json::json;

fn db() -> Connection {
    let db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap();
    initialize(&db).unwrap();
    db
}
fn block(id: &str, parent: Option<&str>, order: u64, kind: BlockKind) -> BlockHeader {
    BlockHeader { id:id.into(), parent:parent.map(str::to_owned), order, kind,
        meta:json!({"label":id}), version:0,length:0,sealed:false,revision:0 }
}
fn root() -> FeedRequest { FeedRequest { scope:"chat".into(), parent:None,cursor:None,floor:0,before:None } }
fn save(db: &mut Connection, h: BlockHeader, bytes: &[u8]) -> bool {
    let tx = db.transaction().unwrap();
    let changed = put(&tx,"chat",h,bytes).unwrap();
    tx.commit().unwrap(); changed
}
fn apply(db: &mut Connection, req: &FeedRequest, page: &FeedPage) {
    let tx = db.transaction().unwrap(); cache_page(&tx,req,page).unwrap(); tx.commit().unwrap();
}
fn download(source: &Connection, cache: &mut Connection, id: &str) -> usize {
    let h = header(source,"chat",id).unwrap().unwrap();
    let tx = cache.transaction().unwrap();
    cache_header(&tx,"chat",&h).unwrap();
    let offset = cached_prefix(&tx,"chat",id).unwrap(); tx.commit().unwrap();
    let mut req = BlockRequest { scope:"chat".into(),id:id.into(),version:h.version,offset,follow:false };
    let mut bytes = 0;
    while req.offset < h.length {
        let range = read(source,&req).unwrap();
        assert!(range.bytes.len() <= BLOCK_CHUNK_BYTES);
        req.offset += range.bytes.len() as u64; bytes += range.bytes.len();
        let tx = cache.transaction().unwrap(); cache_range(&tx,"chat",&range).unwrap(); tx.commit().unwrap();
    }
    bytes
}

#[test]
fn feed_is_direct_children_only_and_never_contains_unrequested_tool_content() {
    let mut source = db();
    save(&mut source,block("tool",None,1,BlockKind::Tool),b"");
    save(&mut source,block("input",Some("tool"),0,BlockKind::Code),&vec![b'x';1024*1024]);
    let page = feed(&source,&root()).unwrap();
    assert_eq!(page.records.len(),1);
    assert!(serde_json::to_vec(&page).unwrap().len() < 1024);
    let mut child = root(); child.parent = Some("tool".into());
    let page = feed(&source,&child).unwrap();
    assert_eq!(page.records.len(),1);
    assert!(serde_json::to_vec(&page).unwrap().len() < 1024,"Even an explicitly requested child feed contains metadata, not content");
}

#[test]
fn appends_are_linear_and_sealing_does_not_resend_or_change_identity() {
    let mut source = db(); let mut cache = db();
    save(&mut source,block("code",None,1,BlockKind::Code),b"");
    let page = feed(&source,&root()).unwrap(); apply(&mut cache,&root(),&page);
    let mut transferred = 0;
    for _ in 0..128 {
        let h = header(&source,"chat","code").unwrap().unwrap();
        let tx = source.transaction().unwrap();
        append(&tx,"chat","code",h.version,h.length,&vec![b'x';256],false).unwrap(); tx.commit().unwrap();
        transferred += download(&source,&mut cache,"code");
    }
    assert_eq!(transferred,32768,"Tool code uses the same suffix path as ordinary text");
    let mut h = header(&source,"chat","code").unwrap().unwrap();
    let version = h.version; h.sealed = true;
    save(&mut source,h,&vec![b'x';32768]);
    assert_eq!(download(&source,&mut cache,"code"),0);
    assert_eq!(header(&source,"chat","code").unwrap().unwrap().version,version);
    assert_eq!(cached_content(&cache,"chat","code").unwrap(),vec![b'x';32768]);
    let count: u64 = source.query_row("SELECT count(*) FROM block_chunks",[],|r|r.get(0)).unwrap();
    assert_eq!(count,1,"Identical sealed chunks deduplicate; abandoned live tails are collected");
}

#[test]
fn unchanged_reconnect_and_process_restart_need_zero_content_bytes() {
    let dir = tempfile::tempdir().unwrap(); let path = dir.path().join("source.sqlite3");
    let mut source = Connection::open(&path).unwrap(); source.execute_batch("PRAGMA foreign_keys=ON; PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL").unwrap(); initialize(&source).unwrap();
    let mut cache = db();
    save(&mut source,block("text",None,0,BlockKind::Text),&vec![b'x';40_001]);
    let page = feed(&source,&root()).unwrap(); apply(&mut cache,&root(),&page);
    assert_eq!(download(&source,&mut cache,"text"),40_001);
    let cursor = page.cursor;
    drop(source);
    let source = Connection::open(path).unwrap(); initialize(&source).unwrap();
    let page = feed(&source,&FeedRequest { cursor:Some(cursor), ..root() }).unwrap();
    assert!(!page.reset && page.records.is_empty());
    assert_eq!(download(&source,&mut cache,"text"),0);
}

#[test]
fn replacing_content_invalidates_only_that_blocks_old_version() {
    let mut source = db(); let mut cache = db();
    save(&mut source,block("a",None,1,BlockKind::Code),b"old text");
    save(&mut source,block("b",None,2,BlockKind::Text),b"keep");
    apply(&mut cache,&root(),&feed(&source,&root()).unwrap());
    download(&source,&mut cache,"a"); download(&source,&mut cache,"b");
    save(&mut source,block("a",None,1,BlockKind::Code),b"new text");
    assert_eq!(download(&source,&mut cache,"a"),8);
    assert_eq!(download(&source,&mut cache,"b"),0);
    assert_eq!(cached_content(&cache,"chat","a").unwrap(),b"new text");
}

#[test]
fn journal_change_does_not_resend_parents_or_siblings() {
    let mut source = db();
    save(&mut source,block("tool",None,1,BlockKind::Tool),b"");
    save(&mut source,block("code",Some("tool"),0,BlockKind::Code),b"a");
    let head = cursor(&source).unwrap();
    save(&mut source,block("code",Some("tool"),0,BlockKind::Code),b"ab");
    assert!(feed(&source,&FeedRequest {cursor:Some(head.clone()),..root()}).unwrap().records.is_empty());
    let children = feed(&source,&FeedRequest {cursor:Some(head),parent:Some("tool".into()),..root()}).unwrap();
    assert_eq!(children.records.len(),1);
}

#[test]
fn interrupted_metadata_transaction_cannot_advance_cursor() {
    let mut source = db(); let mut cache = db();
    save(&mut source,block("a",None,1,BlockKind::Text),b"one");
    let first = feed(&source,&root()).unwrap(); apply(&mut cache,&root(),&first);
    save(&mut source,block("b",None,2,BlockKind::Text),b"two");
    let request = FeedRequest { cursor:Some(first.cursor.clone()),..root() };
    let next = feed(&source,&request).unwrap();
    { let tx = cache.transaction().unwrap(); cache_page(&tx,&request,&next).unwrap(); }
    assert_eq!(cached_feed(&cache,"chat",None).unwrap().unwrap().cursor,first.cursor);
    assert!(header(&cache,"chat","b").unwrap().is_none());
    apply(&mut cache,&request,&next);
    assert!(header(&cache,"chat","b").unwrap().is_some());
}

#[test]
fn late_data_cannot_resurrect_a_deleted_block() {
    let mut source = db(); let mut cache = db();
    save(&mut source,block("a",None,0,BlockKind::Text),b"secret old content");
    let page = feed(&source,&root()).unwrap(); apply(&mut cache,&root(),&page);
    let range = read(&source,&BlockRequest {scope:"chat".into(),id:"a".into(),version:1,offset:0,follow:false}).unwrap();
    let tx = source.transaction().unwrap(); remove(&tx,"chat","a").unwrap(); tx.commit().unwrap();
    let req = FeedRequest { cursor:Some(page.cursor),..root() };
    apply(&mut cache,&req,&feed(&source,&req).unwrap());
    let tx = cache.transaction().unwrap(); cache_range(&tx,"chat",&range).unwrap(); tx.commit().unwrap();
    assert!(header(&cache,"chat","a").unwrap().is_none());
}

#[test]
fn hard_bounds_and_parent_cycles_are_rejected_transactionally() {
    let mut source = db();
    let tx = source.transaction().unwrap();
    let mut h = block("too-big",None,0,BlockKind::Text); h.meta = json!({"text":"x".repeat(MAX_BLOCK_HEADER_BYTES)});
    assert!(put(&tx,"chat",h,b"").is_err());
    assert!(header(&tx,"chat","too-big").unwrap().is_none());
    tx.rollback().unwrap();
    save(&mut source,block("a",None,1,BlockKind::Tool),b"");
    save(&mut source,block("b",Some("a"),2,BlockKind::Tool),b"");
    let tx = source.transaction().unwrap();
    assert!(put(&tx,"chat",block("a",Some("b"),1,BlockKind::Tool),b"").is_err());
}

#[test]
fn bad_hash_does_not_cache_bytes_or_advance_verified_offset() {
    let mut source = db(); let mut cache = db();
    save(&mut source,block("a",None,0,BlockKind::Text),b"good");
    apply(&mut cache,&root(),&feed(&source,&root()).unwrap());
    let mut range = read(&source,&BlockRequest {scope:"chat".into(),id:"a".into(),version:1,offset:0,follow:false}).unwrap();
    range.bytes[0] = b'x';
    let tx = cache.transaction().unwrap();
    assert!(cache_range(&tx,"chat",&range).is_err());
    assert_eq!(cached_prefix(&tx,"chat","a").unwrap(),0);
}

#[test]
fn feed_pages_and_cursors_survive_more_than_one_page_of_changes() {
    let mut source = db();
    let initial = cursor(&source).unwrap();
    for i in 0..100 { save(&mut source,block(&format!("b{i}"),None,i,BlockKind::Text),b"x"); }
    let mut req = FeedRequest { cursor:Some(initial),..root() };
    let mut seen = 0;
    loop {
        let page = feed(&source,&req).unwrap(); seen += page.records.len(); req.cursor = Some(page.cursor);
        if !page.more { break; }
    }
    assert_eq!(seen,100);
    assert!(feed(&source,&req).unwrap().records.is_empty());
    let page = feed(&source,&root()).unwrap();
    assert_eq!(page.records.len(),MAX_FEED_PAGE);
    assert!(page.before.is_some());
    let older = feed(&source,&FeedRequest {before:page.before,..root()}).unwrap();
    assert_eq!(older.records.len(),MAX_FEED_PAGE);
    assert!(older.records.iter().all(|r|matches!(r,BlockRecord::Put{block} if block.order < page.floor)));
}

#[test]
fn hot_blocks_coalesce_without_expiring_quiet_cursors() {
    let mut source=db();
    save(&mut source,block("hot",None,0,BlockKind::Text),b"");
    let before=cursor(&source).unwrap();
    for offset in 0..1000 {
        let tx=source.transaction().unwrap();append(&tx,"chat","hot",1,offset,b"x",false).unwrap();tx.commit().unwrap();
    }
    assert_eq!(source.query_row("SELECT count(*) FROM block_changes",[],|r|r.get::<_,u64>(0)).unwrap(),1);
    let page=feed(&source,&FeedRequest {cursor:Some(before),..root()}).unwrap();
    assert!(!page.reset);assert_eq!(page.records.len(),1);
    assert!(matches!(&page.records[0],BlockRecord::Put {block} if block.length==1000));
}

#[test]
fn history_handles_equal_positions_and_late_lower_order_insertions() {
    let mut source=db();
    for n in 0..99 {save(&mut source,block(&format!("node-{n:03}"),None,i64::MAX as u64,BlockKind::Text),b"");}
    let first=feed(&source,&root()).unwrap();let cursor=first.cursor.clone();let mut page=first;
    let mut ids=std::collections::HashSet::new();
    loop {
        for r in page.records {if let BlockRecord::Put {block}=r {assert!(ids.insert(block.id));}}
        let Some(before)=page.before else {break;};
        page=feed(&source,&FeedRequest {before:Some(before),..root()}).unwrap();
    }
    assert_eq!(ids.len(),99);
    save(&mut source,block("later",None,0,BlockKind::Text),b"new");
    let page=feed(&source,&FeedRequest {cursor:Some(cursor),floor:i64::MAX as u64,..root()}).unwrap();
    assert!(matches!(&page.records[..],[BlockRecord::Put {block}] if block.id=="later"));
}

#[test]
fn finite_history_cannot_skip_live_changes_or_erase_equal_position_siblings() {
    let mut source=db();let mut cache=db();
    for n in 0..70 {save(&mut source,block(&format!("n-{n:03}"),None,10,BlockKind::Text),b"old");}
    let first=feed(&source,&root()).unwrap();apply(&mut cache,&root(),&first);
    save(&mut source,block("n-069",None,10,BlockKind::Text),b"changed");
    let req=FeedRequest {before:first.before.clone(),..root()};
    let older=feed(&source,&req).unwrap();apply(&mut cache,&req,&older);
    assert_eq!(cached_feed(&cache,"chat",None).unwrap().unwrap().cursor,first.cursor);
    assert_eq!(children(&cache,"chat",None).unwrap().len(),64);
    let req=FeedRequest {cursor:Some(first.cursor),..root()};
    apply(&mut cache,&req,&feed(&source,&req).unwrap());
    assert_eq!(header(&cache,"chat","n-069").unwrap().unwrap().length,7);
}

#[test]
fn corrupt_shared_cache_chunk_is_repaired_not_reused_forever() {
    let mut source=db();let mut cache=db();
    for id in ["a","b"] {save(&mut source,block(id,None,0,BlockKind::Text),b"same bytes");download(&source,&mut cache,id);}
    cache.execute("UPDATE block_chunks SET data=x'00'",[]).unwrap();
    assert_eq!(download(&source,&mut cache,"a"),10);
    assert_eq!(cached_content(&cache,"chat","a").unwrap(),b"same bytes");
    assert_eq!(cached_content(&cache,"chat","b").unwrap(),b"same bytes");
}

#[test]
fn staged_import_has_no_feed_changes_until_atomic_publish_and_cleans_up() {
    let mut source=db();save(&mut source,block("file",None,0,BlockKind::File),b"");
    let before=cursor(&source).unwrap();let bytes=vec![42;BLOCK_CHUNK_BYTES*17+123];
    for (n,chunk) in bytes.chunks(BLOCK_CHUNK_BYTES*8).enumerate() {
        let tx=source.transaction().unwrap();stage_append(&tx,"import",(n*BLOCK_CHUNK_BYTES*8) as u64,chunk).unwrap();tx.commit().unwrap();
        assert_eq!(cursor(&source).unwrap(),before);
    }
    let tx=source.transaction().unwrap();let h=publish_stage(&tx,"import","chat","file",json!({"materialized":true})).unwrap();tx.commit().unwrap();
    assert!(h.sealed);assert_eq!(h.version,1);assert_eq!(h.length,bytes.len() as u64);
    assert!(children(&source,STAGING_SCOPE,None).unwrap().is_empty());
    let mut cache=db();assert_eq!(download(&source,&mut cache,"file"),bytes.len());
    assert_eq!(cached_content(&cache,"chat","file").unwrap(),bytes);
    let count=source.query_row("SELECT count(*) FROM block_chunks",[],|r|r.get::<_,u64>(0)).unwrap();
    let tx=source.transaction().unwrap();stage_append(&tx,"abandoned",0,b"discard me").unwrap();discard_staging(&tx).unwrap();tx.commit().unwrap();
    assert_eq!(source.query_row("SELECT count(*) FROM block_chunks",[],|r|r.get::<_,u64>(0)).unwrap(),count);
}
