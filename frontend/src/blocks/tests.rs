use super::*;
use serde_json::json;

struct Fixture {source:Connection,cache:Cache,_root:tempfile::TempDir,lineage:String}
impl Fixture {
    fn new()->Self {
        let source=Connection::open_in_memory().unwrap();source.execute_batch("PRAGMA foreign_keys=ON").unwrap();tau_blocks::initialize(&source).unwrap();
        let root=tempfile::tempdir().unwrap();let cache=Cache::open(&root.path().join("cache.db")).unwrap();
        let lineage=tau_blocks::cursor(&source).unwrap().lineage;cache.configure(&lineage).unwrap();
        Self {source,cache,_root:root,lineage}
    }
    fn put(&mut self,id:&str,parent:Option<&str>,order:u64,kind:BlockKind,meta:serde_json::Value,bytes:&[u8]) {
        let tx=self.source.transaction().unwrap();
        tau_blocks::put(&tx,"chat",BlockHeader {id:id.into(),parent:parent.map(str::to_owned),order,kind,meta,version:0,length:0,sealed:true,revision:0},bytes).unwrap();tx.commit().unwrap();
    }
    fn page(&self,parent:Option<&str>,before:Option<FeedPosition>) {
        let req=self.cache.feed_request("chat",parent,before).unwrap();let page=tau_blocks::feed(&self.source,&req).unwrap();self.cache.page(&self.lineage,&req,&page).unwrap();
    }
    fn body(&self,id:&str) {
        loop {
            let req=self.cache.block_request("chat",id).unwrap();let range=tau_blocks::read(&self.source,&req).unwrap();
            self.cache.header(&self.lineage,"chat",&range.header).unwrap();
            if range.bytes.is_empty() {break;}
            self.cache.range(&self.lineage,"chat",&range).unwrap();
        }
    }
}
fn event(id:&str,order:u64,kind:&str)->serde_json::Value {
    json!({"event":{"id":id,"entryId":id,"order":order,"phase":"saved","origin":{},"role":"assistant","kind":kind,"text":"","isError":false,"toolCallId":id,"toolName":"bash"},"toolState":"completed"})
}

#[test]
fn queue_directory_paginates_fully_and_partial_text_is_not_editable() {
    let mut f=Fixture::new();
    f.put(QUEUE,None,i64::MAX as u64,BlockKind::Queue,json!({}),&serde_json::to_vec(&QueueState::native()).unwrap());
    for n in 0..99 {
        let id=format!("request-{n}");let meta=json!({"request":tau_protocol::QueuedRequest {request_id:id.clone(),revision:0,kind:"steer".into(),text:String::new(),images:0,timestamp_ms:None}});
        f.put(&format!("queued:{id}"),Some(QUEUE),n,BlockKind::Text,meta,b"whole message");
    }
    f.page(None,None);f.body(QUEUE);f.page(Some(QUEUE),None);
    let view=f.cache.snapshot("chat").unwrap().unwrap();assert!(!view.snapshot.queue.available);assert_eq!(view.snapshot.queue.requests.len(),MAX_FEED_PAGE);
    loop {
        let plan=f.cache.plan("chat",&LocalChat::default(),&[]).unwrap();
        if plan.older.is_empty() {break;}
        for (parent,before) in plan.older {f.page(Some(&parent),Some(before));}
    }
    let view=f.cache.snapshot("chat").unwrap().unwrap();assert!(view.snapshot.queue.available);assert_eq!(view.snapshot.queue.requests.len(),99);
    assert_eq!(view.incomplete.len(),99);
    f.body("queued:request-0");let view=f.cache.snapshot("chat").unwrap().unwrap();
    assert!(!view.incomplete.contains("queued:request-0"));assert_eq!(view.snapshot.queue.requests[0].text,"whole message");
}

#[test]
fn disclosure_interests_are_per_group_and_large_input_is_explicit() {
    let mut f=Fixture::new();
    f.put("a",None,0,BlockKind::Tool,event("a",0,"tool"),b"");
    f.put("a/input",Some("a"),0,BlockKind::Code,json!({"inputFor":"a"}),&vec![b'a';20000]);
    f.put("answer",None,2,BlockKind::Text,event("answer",2,"text"),b"answer");
    f.put("b",None,4,BlockKind::Tool,event("b",4,"tool"),b"");
    f.put("b/input",Some("b"),0,BlockKind::Code,json!({"inputFor":"b"}),&vec![b'b';20000]);
    f.page(None,None);f.page(Some("a"),None);f.page(Some("b"),None);
    let mut local=LocalChat::default();local.expansion.insert("details:b".into(),true);
    local.expansion.insert("tool:a".into(),true);local.expansion.insert("tool:b".into(),true);
    let plan=f.cache.plan("chat",&local,&[]).unwrap();
    assert!(!plan.parents.contains(&Some("a".into())));assert!(plan.parents.contains(&Some("b".into())));
    assert!(!plan.blocks.iter().any(|(id,_)|id.ends_with("/input")));
    let view=f.cache.snapshot("chat").unwrap().unwrap();
    let tools=crate::details::Tools::new(view.snapshot.events.iter()).with_lengths(&view.lengths).with_states(&view.states);
    let lines=tools.lines(&[view.snapshot.events.iter().find(|e|e.id=="b").unwrap()],&local);
    assert!(lines.iter().any(|line|line.label=="Input" && line.toggle==Some(false)),"An unfetched input still has an expansion control");
    local.expansion.insert("tool:b:Input".into(),true);
    let plan=f.cache.plan("chat",&local,&[]).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="b/input"));assert!(!plan.blocks.iter().any(|(id,_)|id=="a/input"));
    let plan=f.cache.plan("chat",&LocalChat::default(),&["a".into()]).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="a/input"));
    assert!(f.cache.copy_ready("chat",&["a".into()]).unwrap().is_none());
    f.body("a/input");let copied=f.cache.copy_ready("chat",&["a".into()]).unwrap().unwrap();assert!(copied.contains(&"a".repeat(20000)));assert!(!copied.contains("Loading"));
}

#[test]
fn text_prefix_handles_split_utf8_and_old_connections_cannot_pollute_new_cache() {
    let mut f=Fixture::new();let text=format!("{}😀","x".repeat(BLOCK_CHUNK_BYTES-1));
    f.put("text",None,0,BlockKind::Text,event("text",0,"text"),text.as_bytes());f.page(None,None);
    let range=tau_blocks::read(&f.source,&f.cache.block_request("chat","text").unwrap()).unwrap();
    f.cache.range(&f.lineage,"chat",&range).unwrap();let view=f.cache.snapshot("chat").unwrap().unwrap();
    assert_eq!(view.snapshot.events[0].text.len(),BLOCK_CHUNK_BYTES-1);assert!(view.incomplete.contains("text"));
    f.body("text");let view=f.cache.snapshot("chat").unwrap().unwrap();assert_eq!(view.snapshot.events[0].text,text);assert!(view.incomplete.is_empty());
    f.cache.configure("other-source").unwrap();assert!(f.cache.range(&f.lineage,"chat",&range).is_err());
    assert!(f.cache.header(&f.lineage,"chat",&range.header).is_err());assert!(f.cache.snapshot("chat").unwrap().is_none());
}

#[test]
fn viewport_limits_body_interests_and_copy_waits_for_sealed_content() {
    let mut f=Fixture::new();
    for n in 0..100 {let id=format!("text-{n}");f.put(&id,None,n,BlockKind::Text,event(&id,n,"text"),b"body");}
    f.page(None,None);
    while let Some(before)=f.cache.history_cursor("chat").unwrap() {f.page(None,Some(before));}
    let plan=f.cache.plan("chat",&LocalChat::default(),&[]).unwrap();
    assert!(plan.blocks.len()<=31);assert!(plan.blocks.iter().any(|(id,_)|id=="text-99"));
    let visible=BTreeSet::from(["text-3".into(),"text-4".into()]);
    let plan=f.cache.plan_visible("chat",&LocalChat::default(),&[],Some(&visible)).unwrap();
    assert_eq!(plan.blocks.iter().map(|(id,_)|id.clone()).collect::<BTreeSet<_>>(),BTreeSet::from([QUEUE.into(),"text-3".into(),"text-4".into()]));
    let tx=f.source.transaction().unwrap();
    tau_blocks::put(&tx,"chat",BlockHeader {id:"live".into(),parent:None,order:200,kind:BlockKind::Thinking,meta:event("live",200,"thinking"),version:0,length:0,sealed:false,revision:0},b"observed prefix").unwrap();tx.commit().unwrap();
    f.page(None,None);f.body("live");assert!(f.cache.copy_ready("chat",&["live".into()]).unwrap().is_none());
    let tx=f.source.transaction().unwrap();let mut h=tau_blocks::header(&tx,"chat","live").unwrap().unwrap();h.sealed=true;tau_blocks::set_header(&tx,"chat",h).unwrap();tx.commit().unwrap();
    f.page(None,None);assert!(f.cache.copy_ready("chat",&["live".into()]).unwrap().unwrap().contains("observed prefix"));
}

#[test]
fn copy_interest_advances_in_bounded_cohorts_instead_of_starving_after_thirty_cards() {
    let mut f=Fixture::new();let ids=(0..100).map(|n|format!("thinking-{n}")).collect::<Vec<_>>();
    for (n,id) in ids.iter().enumerate() {f.put(id,None,n as u64,BlockKind::Thinking,event(id,n as u64,"thinking"),b"thought");}
    f.page(None,None);while let Some(before)=f.cache.history_cursor("chat").unwrap() {f.page(None,Some(before));}
    let mut fetched=BTreeSet::new();
    for expected in [30,30,30,10] {
        assert!(f.cache.copy_ready("chat",&ids).unwrap().is_none());
        let plan=f.cache.plan("chat",&LocalChat::default(),&ids).unwrap();
        let bodies=plan.blocks.iter().filter(|(id,_)|id!=QUEUE).map(|(id,_)|id.clone()).collect::<Vec<_>>();assert_eq!(bodies.len(),expected);
        for id in bodies {assert!(fetched.insert(id.clone()));f.body(&id);}
    }
    assert_eq!(fetched.len(),100);assert_eq!(f.cache.copy_ready("chat",&ids).unwrap().unwrap().matches("thought").count(),100);
}

#[test]
fn sqlite_full_and_cross_handle_reset_never_advance_a_verified_prefix() {
    let mut f=Fixture::new();f.put("body",None,0,BlockKind::Text,event("body",0,"text"),&vec![7;BLOCK_CHUNK_BYTES*2]);f.page(None,None);
    let req=f.cache.block_request("chat","body").unwrap();let range=tau_blocks::read(&f.source,&req).unwrap();
    {let db=f.cache.db.lock().unwrap();let pages:u64=db.query_row("PRAGMA page_count",[],|r|r.get(0)).unwrap();db.pragma_update(None,"max_page_count",pages).unwrap();}
    let error=f.cache.range(&f.lineage,"chat",&range).unwrap_err();assert!(error.to_string().contains("full"),"{error:#}");
    assert_eq!(f.cache.block_request("chat","body").unwrap().offset,0);
    let old=f.cache.epoch();let second=Cache::open(&f._root.path().join("cache.db")).unwrap();second.clear().unwrap();
    assert!(f.cache.header_at(&f.lineage,"chat",&range.header,old).is_err());assert!(f.cache.range_at(&f.lineage,"chat",&range,old).is_err());
    assert!(tau_blocks::header(&f.cache.db.lock().unwrap(),"chat","body").unwrap().is_none());
}

#[test]
fn sparse_projection_and_retained_previews_are_bounded_across_many_updates() {
    let mut f=Fixture::new();let bytes=vec![b'x';300*1024];
    for i in 0..128 {let id=format!("e{i:03}");f.put(&id,None,i,BlockKind::Text,event(&id,i,"text"),&bytes);}
    f.page(None,None);
    loop {let before=f.cache.history_cursor("chat").unwrap();if let Some(before)=before {f.page(None,Some(before));} else {break;}}
    let mut feed=crate::feed::Feed::default();feed.native_view(f.cache.changes("chat",None,true).unwrap().unwrap()).unwrap();
    for i in 0..128 {
        let id=format!("e{i:03}");f.body(&id);let visible=BTreeSet::from([id.clone()]);
        let view=f.cache.changes("chat",Some(&visible),false).unwrap().unwrap();assert!(view.partial);assert_eq!(view.snapshot.events.len(),1);
        assert!(view.incomplete.contains(&id));assert!(view.snapshot.events[0].text.contains("Preview limited"));
        feed.native_view(view).unwrap();
        assert!(feed.events.values().map(|e|e.text.len()).sum::<usize>()<=8*1024*1024+16000);
    }
    assert_eq!(feed.events.len(),128);assert!(feed.event("e000").unwrap().text.len()<100);
    let text=f.cache.copy_ready("chat",&["e127".into()]).unwrap().unwrap();assert_eq!(text.as_bytes(),bytes);
    // Native full views replace the authoritative cache cut, unlike the legacy
    // display adapter's overlapping history merge.
    f.cache.clear().unwrap();f.page(None,None);feed.native_view(f.cache.changes("chat",None,true).unwrap().unwrap()).unwrap();
    assert_eq!(feed.events.len(),MAX_FEED_PAGE);
}

#[test]
fn copy_preflights_all_metadata_even_when_an_earlier_body_is_missing() {
    let mut f=Fixture::new();
    for (i,id) in ["a","b"].into_iter().enumerate() {let mut meta=event(id,i as u64,"text");meta["fullEvent"]=json!({"id":format!("{id}/meta"),"hash":"a".repeat(64),"length":40*1024*1024});f.put(id,None,i as u64,BlockKind::Text,meta,b"missing");}
    f.page(None,None);
    assert!(f.cache.copy_ready("chat",&["a".into(),"b".into()]).unwrap_err().to_string().contains("64 MiB"));
    let plan=f.cache.plan("chat",&LocalChat::default(),&[]).unwrap();assert!(!plan.blocks.iter().any(|(id,_)|id.ends_with("/meta")));
    let view=f.cache.preview("chat",None).unwrap().unwrap();assert!(view.incomplete.contains("a"));
}

#[test]
fn giant_previews_stop_prefetching_and_copy_can_target_a_closed_child() {
    let mut f=Fixture::new();let bytes=vec![b'x';300*1024];f.put("tool",None,0,BlockKind::Tool,event("tool",0,"tool"),b"");
    let mut meta=event("result",1,"text");meta["event"]["role"]=json!("tool");f.put("result",Some("tool"),1,BlockKind::Code,meta,&bytes);
    f.page(None,None);f.page(Some("tool"),None);
    let plan=f.cache.plan_visible("chat",&LocalChat::default(),&["result".into()],Some(&BTreeSet::new())).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="result"));
    let mut local=LocalChat {details_default:true,..Default::default()};local.expansion.insert("tool:tool".into(),true);local.expansion.insert("tool:tool:Output".into(),true);
    for _ in 0..16 {let req=f.cache.block_request("chat","result").unwrap();let range=tau_blocks::read(&f.source,&req).unwrap();f.cache.range(&f.lineage,"chat",&range).unwrap();}
    let plan=f.cache.plan("chat",&local,&[]).unwrap();assert!(!plan.blocks.iter().any(|(id,_)|id=="result"));
    let plan=f.cache.plan("chat",&local,&["result".into()]).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="result"));f.body("result");let text=f.cache.copy_ready("chat",&["result".into()]).unwrap().unwrap();assert_eq!(text.bytes().filter(|b|*b==b'x').count(),bytes.len());assert!(!text.contains("Preview limited"));
}

#[test]
fn cache_migrates_without_losing_verified_bytes_and_rejects_future_versions() {
    let mut f=Fixture::new();f.put("kept",None,0,BlockKind::Text,event("kept",0,"text"),b"verified");f.page(None,None);f.body("kept");
    f.cache.db.lock().unwrap().execute_batch("DROP TABLE replica_epoch; PRAGMA user_version=1").unwrap();
    let cache=Cache::open(&f._root.path().join("cache.db")).unwrap();assert_eq!(cache.epoch(),0);assert_eq!(cache.copy_ready("chat",&["kept".into()]).unwrap().unwrap(),"verified");
    cache.db.lock().unwrap().execute_batch("PRAGMA user_version=999").unwrap();assert!(Cache::open(&f._root.path().join("cache.db")).is_err());assert_eq!(tau_blocks::cached_content(&cache.db.lock().unwrap(),"chat","kept").unwrap(),b"verified");
}

#[test]
fn native_tool_pairing_uses_block_parents_not_reused_provider_call_ids() {
    let mut f=Fixture::new();
    for (id,order) in [("a",0),("b",2)] {let mut meta=event(id,order,"tool");meta["event"]["toolCallId"]=json!("provider-reused");f.put(id,None,order,BlockKind::Tool,meta,b"");}
    for (id,parent,order,text) in [("one","a",1,b"first".as_slice()),("two","b",3,b"second".as_slice())] {let mut meta=event(id,order,"text");meta["event"]["role"]=json!("tool");meta["event"]["toolCallId"]=json!("provider-reused");f.put(id,Some(parent),order,BlockKind::Code,meta,text);}
    f.page(None,None);f.page(Some("a"),None);f.page(Some("b"),None);f.body("one");f.body("two");
    let view=f.cache.snapshot("chat").unwrap().unwrap();let tools=crate::details::Tools::new(view.snapshot.events.iter());
    let a=view.snapshot.events.iter().find(|e|e.id=="a").unwrap();let text=tools.copy(&[a]);assert!(text.contains("first"));assert!(!text.contains("second"));
}

#[test]
fn visible_file_captions_load_full_metadata_without_requesting_binary_payloads() {
    let mut f=Fixture::new();let mut meta=event("card",0,"text");meta["event"]["role"]=json!("tool");meta["event"]["attachment"]=json!({"fileName":"report.txt","kind":"file","caption":"short","size":12});
    let caption="🦀".repeat(1024);let mut full=meta["event"].clone();full["attachment"]["caption"]=json!(caption);let bytes=serde_json::to_vec(&full).unwrap();
    meta["fullEvent"]=json!({"id":"card/meta","length":bytes.len(),"hash":blake3::hash(&bytes).to_hex().to_string()});
    f.put("card",None,0,BlockKind::Code,meta,b"");f.put("card/meta",Some("card"),0,BlockKind::State,json!({"eventMetadata":true}),&bytes);f.page(None,None);
    let visible=BTreeSet::from(["card".into()]);let plan=f.cache.plan_visible("chat",&LocalChat::default(),&[],Some(&visible)).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="card/meta"));assert!(!plan.blocks.iter().any(|(id,_)|id.starts_with("file:")));
    f.body("card/meta");let view=f.cache.preview("chat",Some(&visible)).unwrap().unwrap();assert_eq!(view.snapshot.events[0].attachment.as_ref().unwrap().caption.as_deref(),Some(caption.as_str()));
}

#[test]
fn attachment_history_paging_clears_loading_without_fetching_closed_tools_or_files() {
    let mut f = Fixture::new();
    for n in 0..100 {
        let id = format!("event-{n}");
        let mut meta = event(&id, n, "tool");
        let kind = if n % 20 == 0 {
            meta["event"]["role"] = json!("tool");
            meta["event"]["kind"] = json!("text");
            meta["event"]["attachment"] = json!({"fileName":format!("file-{n}.txt"),"kind":"file","size":12});
            BlockKind::Code
        } else { BlockKind::Tool };
        f.put(&id, None, n, kind, meta, b"unread output");
        f.put(&format!("file:{id}"), Some(&id), 1, BlockKind::File,
            json!({"attachment":{"kind":"file"}}), b"file payload");
    }
    f.page(None, None);
    let visible = BTreeSet::new();
    let mut feed = crate::feed::Feed::default();
    feed.native_view(f.cache.changes("chat", Some(&visible), true).unwrap().unwrap()).unwrap();
    let mut pages = 1;
    while let Some(before) = f.cache.history_cursor("chat").unwrap() {
        feed.loading = true;
        f.cache.changed("chat", "event-99", false);
        feed.native_view(f.cache.changes("chat", Some(&visible), false).unwrap().unwrap()).unwrap();
        assert!(feed.loading, "unrelated content progress does not complete history loading");
        f.page(None, Some(before));
        let view = f.cache.changes("chat", Some(&visible), false).unwrap().unwrap();
        feed.native_view(view).unwrap();
        assert!(!feed.loading, "the next older page remains reachable");
        pages += 1;
        assert!(pages < 10);
    }
    assert!(pages > 2);
    let files = feed.events.values().rev().filter(|e| e.attachment.is_some()).collect::<Vec<_>>();
    assert_eq!(files.iter().map(|e| e.order).collect::<Vec<_>>(), vec![80, 60, 40, 20, 0]);
    let visible = files.iter().map(|e| e.id.clone()).collect::<BTreeSet<_>>();
    let plan = f.cache.plan_visible("chat", &LocalChat::default(), &[], Some(&visible)).unwrap();
    assert_eq!(plan.parents, BTreeSet::from([None, Some(QUEUE.into())]));
    assert_eq!(plan.blocks.iter().map(|(id, _)| id.as_str()).collect::<Vec<_>>(), vec![QUEUE]);
    let db = f.cache.db.lock().unwrap();
    for id in &visible {
        assert!(tau_blocks::cached_content(&db, "chat", id).unwrap().is_empty());
        assert!(tau_blocks::header(&db, "chat", &format!("file:{id}")).unwrap().is_none());
    }
}
