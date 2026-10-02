use super::*;
use serde_json::json;

pub(crate) struct Fixture {pub(crate) source:Connection,pub(crate) cache:Cache,pub(crate) _root:tempfile::TempDir,pub(crate) lineage:String}
impl Fixture {
    pub(crate) fn new()->Self {
        let source=Connection::open_in_memory().unwrap();source.execute_batch("PRAGMA foreign_keys=ON").unwrap();tau_block_store::initialize(&source).unwrap();
        let root=tempfile::tempdir().unwrap();let cache=Cache::open(&root.path().join("cache.db")).unwrap();
        let lineage=tau_block_store::cursor(&source).unwrap().lineage;cache.configure(&lineage).unwrap();
        Self {source,cache,_root:root,lineage}
    }
    pub(crate) fn put(&mut self,id:&str,parent:Option<&str>,order:u64,kind:BlockKind,meta:serde_json::Value,bytes:&[u8]) {
        let tx=self.source.transaction().unwrap();
        tau_block_store::put(&tx,"chat",BlockHeader {id:id.into(),parent:parent.map(str::to_owned),order,kind,meta,version:0,length:0,sealed:true,revision:0},bytes).unwrap();tx.commit().unwrap();
    }
    pub(crate) fn page(&self,parent:Option<&str>,before:Option<FeedPosition>) {
        let req=self.cache.feed_request("chat",parent,before).unwrap();let page=tau_block_store::feed(&self.source,&req).unwrap();self.cache.page(&self.lineage,&req,&page).unwrap();
    }
    pub(crate) fn chunk(&self,id:&str)->bool {
        let req=self.cache.block_request("chat",id).unwrap();let range=tau_block_store::read(&self.source,&req).unwrap();
        self.cache.header(&self.lineage,"chat",&range.header).unwrap();
        if range.bytes.is_empty() {return false;}
        self.cache.range(&self.lineage,"chat",&range).unwrap();true
    }
    pub(crate) fn body(&self,id:&str) {while self.chunk(id) {}}

}
fn event(id:&str,order:u64,kind:&str)->serde_json::Value {
    json!({"event":{"id":id,"entryId":id,"order":order,"phase":"saved","origin":{},"role":"assistant","kind":kind,"text":"","isError":false,"toolCallId":id,"toolName":"bash"},"toolState":"completed"})
}

#[test]
fn body_identity_availability_and_authored_loading_text_are_independent() {
    let mut f = Fixture::new();
    f.put("text", None, 0, BlockKind::Text, event("text", 0, "text"), "Loading…".as_bytes());
    f.put("tool", None, 1, BlockKind::Tool, event("tool", 1, "tool"), b"");
    f.page(None, None);
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    assert!(view.events[0].text.is_empty());
    assert!(view.bodies["text"].missing());
    assert!(view.bodies["tool"].reference.is_none());
    assert!(view.incomplete.contains("tool"), "unknown input is not completed empty input");
    let original = view.bodies["text"].reference.clone().unwrap();
    assert_eq!((&*original.source, &*original.scope, &*original.id), (&*f.lineage, "chat", "text"));
    assert!(original.sealed);
    f.body("text");
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    assert_eq!(view.events[0].text, "Loading…");
    assert!(view.bodies["text"].complete());
    assert_eq!(view.bodies["text"].reference.as_ref(), Some(&original));
    assert_eq!(f.cache.copy_ready("chat", &["text".into()]).unwrap().unwrap(), "Loading…");
    f.put("text", None, 0, BlockKind::Text, event("text", 0, "text"), b"replacement");
    f.page(None, None);
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    assert!(view.bodies["text"].missing());
    assert_ne!(view.bodies["text"].reference.as_ref().unwrap().version, original.version);
}

#[test]
fn queue_directory_paginates_fully_and_partial_text_is_not_editable() {
    let mut f=Fixture::new();
    f.put(QUEUE,None,i64::MAX as u64,BlockKind::Queue,json!({}),&serde_json::to_vec(&QueueState::native()).unwrap());
    for n in 0..99 {
        let id=format!("request-{n}");let meta=json!({"request":tau_net::QueuedRequest {request_id:id.clone(),revision:0,kind:"steer".into(),text:String::new(),images:0,timestamp_ms:None}});
        f.put(&format!("queued:{id}"),Some(QUEUE),n,BlockKind::Text,meta,b"whole message");
    }
    f.page(None,None);f.body(QUEUE);f.page(Some(QUEUE),None);
    let view=f.cache.snapshot("chat").unwrap().unwrap();assert!(!view.queue.available);assert_eq!(view.queue.requests.len(),MAX_FEED_PAGE);
    loop {
        let plan=f.cache.plan("chat",&LocalChat::default(),&[]).unwrap();
        if plan.older.is_empty() {break;}
        for (parent,before) in plan.older {f.page(Some(&parent),Some(before));}
    }
    let view=f.cache.snapshot("chat").unwrap().unwrap();assert!(view.queue.available);assert_eq!(view.queue.requests.len(),99);
    assert_eq!(view.incomplete.len(),99);
    f.body("queued:request-0");let view=f.cache.snapshot("chat").unwrap().unwrap();
    assert!(!view.incomplete.contains("queued:request-0"));assert_eq!(view.queue.requests[0].text,"whole message");
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
    let range=tau_block_store::read(&f.source,&f.cache.block_request("chat","text").unwrap()).unwrap();
    f.cache.range(&f.lineage,"chat",&range).unwrap();let view=f.cache.snapshot("chat").unwrap().unwrap();
    assert_eq!(view.events[0].text.len(),BLOCK_CHUNK_BYTES-1);assert!(view.incomplete.contains("text"));
    f.body("text");let view=f.cache.snapshot("chat").unwrap().unwrap();assert_eq!(view.events[0].text,text);assert!(view.incomplete.is_empty());
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
    tau_block_store::put(&tx,"chat",BlockHeader {id:"live".into(),parent:None,order:200,kind:BlockKind::Thinking,meta:event("live",200,"thinking"),version:0,length:0,sealed:false,revision:0},b"observed prefix").unwrap();tx.commit().unwrap();
    f.page(None,None);f.body("live");assert!(f.cache.copy_ready("chat",&["live".into()]).unwrap().is_none());
    let tx=f.source.transaction().unwrap();let mut h=tau_block_store::header(&tx,"chat","live").unwrap().unwrap();h.sealed=true;tau_block_store::set_header(&tx,"chat",h).unwrap();tx.commit().unwrap();
    f.page(None,None);assert!(f.cache.copy_ready("chat",&["live".into()]).unwrap().unwrap().contains("observed prefix"));
}

#[test]
fn viewport_body_admission_advances_past_cached_rows_and_closed_tools() {
    for mode in ["text", "closed tools", "open tools"] {
        let mut f = Fixture::new();
        let visible = (0..60).map(|n| format!("row-{n:03}")).collect::<BTreeSet<_>>();
        let mut local = LocalChat::default();
        for (n, id) in visible.iter().enumerate() {
            let tool = mode != "text" && n >= 30;
            f.put(id, None, n as u64, if tool { BlockKind::Tool } else { BlockKind::Text },
                event(id, n as u64, if tool { "tool" } else { "text" }), if tool { b"" } else { b"body" });
            if tool {
                f.put(&tool_input_id(id), Some(id), 0, BlockKind::Code, json!({"inputFor":id}), b"body");
                let result = format!("{id}/result");
                let mut meta = event(&result, 60+n as u64, "text"); meta["event"]["role"] = json!("tool");
                f.put(&result, Some(id), 1, BlockKind::Code, meta, b"result");
                if mode == "open tools" { local.expansion.insert(format!("tool:{id}"), true); }
            }
        }
        f.page(None, None);
        while let Some(before) = f.cache.history_cursor("chat").unwrap() { f.page(None, Some(before)); }
        for id in visible.iter().skip(30) { if mode != "text" { f.page(Some(id), None); } }
        let mut fetched = BTreeSet::new();
        for _ in 0..3 {
            let plan = f.cache.plan_visible("chat", &local, &[], Some(&visible)).unwrap();
            let missing = plan.blocks.iter().filter(|(id, head)| id != QUEUE
                && head.is_none_or(|(_, length, _, stored)| stored < length)).map(|(id, _)| id.clone()).collect::<Vec<_>>();
            let groups = missing.iter().map(|id| tau_block_store::header(&f.source,"chat",id).unwrap().unwrap().parent.unwrap_or(id.clone())).collect::<BTreeSet<_>>();
            assert!(groups.len() <= 30, "The active root cohort remains bounded");
            for id in missing { fetched.insert(id.clone()); f.body(&id); }
        }
        let expected = match mode { "closed tools" => 30, "open tools" => 90, _ => 60 };
        assert_eq!(fetched.len(), expected, "Cached rows and tool disclosures must not strand visible loading rows: {mode}");
        let view = f.cache.snapshot("chat").unwrap().unwrap();
        for id in visible.iter().take(if mode == "closed tools" { 30 } else { 60 }) {
            assert_eq!(view.events.iter().find(|e| &e.id == id).unwrap().text, "body");
        }
        if mode == "closed tools" {
            assert!(fetched.iter().all(|id| !id.contains('/')), "Closed tools still cannot fetch child bytes");
        }
    }
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
    let req=f.cache.block_request("chat","body").unwrap();let range=tau_block_store::read(&f.source,&req).unwrap();
    {let db=f.cache.db.lock().unwrap();let pages:u64=db.query_row("PRAGMA page_count",[],|r|r.get(0)).unwrap();db.pragma_update(None,"max_page_count",pages).unwrap();}
    let error=f.cache.range(&f.lineage,"chat",&range).unwrap_err();assert!(error.to_string().contains("full"),"{error:#}");
    assert_eq!(f.cache.block_request("chat","body").unwrap().offset,0);
    let old=f.cache.epoch();let second=Cache::open(&f._root.path().join("cache.db")).unwrap();second.clear().unwrap();
    assert!(f.cache.header_at(&f.lineage,"chat",&range.header,old).is_err());assert!(f.cache.range_at(&f.lineage,"chat",&range,old).is_err());
    assert!(tau_block_store::header(&f.cache.db.lock().unwrap(),"chat","body").unwrap().is_none());
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
        let view=f.cache.changes("chat",Some(&visible),false).unwrap().unwrap();assert!(view.partial);assert_eq!(view.events.len(),1);
        assert!(view.incomplete.contains(&id));assert!(view.bodies[&id].limited);assert!(!view.events[0].text.contains("Preview limited"));
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
    for _ in 0..16 {let req=f.cache.block_request("chat","result").unwrap();let range=tau_block_store::read(&f.source,&req).unwrap();f.cache.range(&f.lineage,"chat",&range).unwrap();}
    let plan=f.cache.plan("chat",&local,&[]).unwrap();assert!(!plan.blocks.iter().any(|(id,_)|id=="result"));
    let plan=f.cache.plan("chat",&local,&["result".into()]).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="result"));f.body("result");let text=f.cache.copy_ready("chat",&["result".into()]).unwrap().unwrap();assert_eq!(text.bytes().filter(|b|*b==b'x').count(),bytes.len());assert!(!text.contains("Preview limited"));
}

#[test]
fn cache_migrates_without_losing_verified_bytes_and_rejects_future_versions() {
    let mut f=Fixture::new();f.put("kept",None,0,BlockKind::Text,event("kept",0,"text"),b"verified");f.page(None,None);f.body("kept");
    f.cache.db.lock().unwrap().execute_batch("DROP TABLE replica_epoch; PRAGMA user_version=1").unwrap();
    let cache=Cache::open(&f._root.path().join("cache.db")).unwrap();assert_eq!(cache.epoch(),0);assert_eq!(cache.copy_ready("chat",&["kept".into()]).unwrap().unwrap(),"verified");
    // Echo reuse was an additive version-2 migration. Failure must roll
    // back the new objects, preserving both verified bytes and the old stamp.
    cache.db.lock().unwrap().execute_batch("DROP TABLE local_echoes; DROP INDEX block_body_hash;
        CREATE TRIGGER fail_migration BEFORE UPDATE ON block_limits BEGIN SELECT RAISE(ABORT,'migration blocked'); END").unwrap();
    assert!(Cache::open(&f._root.path().join("cache.db")).is_err());
    {
        let db = cache.db.lock().unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM sqlite_schema WHERE name='local_echoes'", [], |r|r.get::<_,u32>(0)).unwrap(), 0);
        assert_eq!(tau_block_store::cached_content(&db,"chat","kept").unwrap(), b"verified");
        db.execute_batch("DROP TRIGGER fail_migration").unwrap();
    }
    let current = Cache::open(&f._root.path().join("cache.db")).unwrap();
    assert_eq!(current.copy_ready("chat",&["kept".into()]).unwrap().unwrap(), "verified");
    assert_eq!(current.db.lock().unwrap().query_row("SELECT count(*) FROM sqlite_schema WHERE name IN ('local_echoes','block_body_hash')", [], |r|r.get::<_,u32>(0)).unwrap(), 2);
    cache.db.lock().unwrap().execute_batch("PRAGMA user_version=999").unwrap();assert!(Cache::open(&f._root.path().join("cache.db")).is_err());assert_eq!(tau_block_store::cached_content(&cache.db.lock().unwrap(),"chat","kept").unwrap(),b"verified");
}

#[test]
fn native_tool_membership_preserves_provider_ids_for_display_copy_and_demand() {
    let mut f=Fixture::new();
    for (id,order) in [("a",0),("b",3)] {
        let mut meta=event(id,order,"tool");meta["event"]["toolCallId"]=json!("provider-reused");
        f.put(id,None,order,BlockKind::Tool,meta,b"");
    }
    for (id,parent,order,text,error) in [
        ("one",Some("a"),1,"first",false), ("error",Some("a"),2,"failure",true),
        ("two",Some("b"),4,"second",false), ("orphan",None,5,"alone",false),
    ] {
        let mut meta=event(id,order,"text");meta["event"]["role"]=json!("tool");
        meta["event"]["toolCallId"]=json!("provider-reused");meta["event"]["isError"]=json!(error);
        f.put(id,parent,order,BlockKind::Code,meta,text.repeat(400).as_bytes());
    }
    f.page(None,None);
    assert!(f.cache.copy_ready("chat",&["a".into()]).unwrap().is_none(), "Unknown children are not an empty completed tool");
    f.page(Some("a"),None);f.page(Some("b"),None);
    for id in ["one","error","two","orphan"] {f.body(id);}
    let view=f.cache.snapshot("chat").unwrap().unwrap();
    assert!(view.events.iter().all(|e|e.tool_call_id.as_deref()==Some("provider-reused")),
        "Native membership must not rewrite provider metadata");
    let mut local=LocalChat {details_default:true,..Default::default()};
    local.expansion.extend([("tool:a".into(),true),("tool:b".into(),true),("tool:a:Error".into(),true),("tool:b:Output".into(),true)]);
    let copied=f.cache.copy_ready("chat",&["a".into()]).unwrap().unwrap();
    assert!(copied.contains("Output\nfirst") && copied.contains("Error\nfailure"));
    assert!(!copied.contains("second") && !copied.contains("alone"));
    assert!(f.cache.copy_ready("chat",&["orphan".into()]).unwrap().unwrap().contains("alone"));
    local.expansion.insert("tool:b".into(),false);
    let plan=f.cache.plan("chat",&local,&[]).unwrap();
    assert!(plan.parents.contains(&Some("a".into())) && !plan.parents.contains(&Some("b".into())));
    assert!(plan.blocks.iter().any(|(id,_)|id=="one") && plan.blocks.iter().any(|(id,_)|id=="error"), "The visible Error section contains every native result, not only error-marked children");
    assert!(!plan.blocks.iter().any(|(id,_)|id=="two" || id=="orphan"));
    local.expansion.insert("tool:a:Error".into(),false);
    assert!(f.cache.plan("chat",&local,&[]).unwrap().blocks.iter().all(|(id,_)|id==QUEUE),
        "Closed body sections do not request their large results");
}

#[test]
fn visible_file_captions_load_full_metadata_without_requesting_binary_payloads() {
    let mut f=Fixture::new();let mut meta=event("card",0,"text");meta["event"]["role"]=json!("tool");meta["event"]["attachment"]=json!({"fileName":"report.txt","kind":"file","caption":"short","size":12});
    let caption="🦀".repeat(1024);let mut full=meta["event"].clone();full["attachment"]["caption"]=json!(caption);let bytes=serde_json::to_vec(&full).unwrap();
    meta["fullEvent"]=json!({"id":"card/meta","length":bytes.len(),"hash":blake3::hash(&bytes).to_hex().to_string()});
    f.put("card",None,0,BlockKind::Code,meta,b"");f.put("card/meta",Some("card"),0,BlockKind::State,json!({"eventMetadata":true}),&bytes);f.page(None,None);
    let visible=BTreeSet::from(["card".into()]);let plan=f.cache.plan_visible("chat",&LocalChat::default(),&[],Some(&visible)).unwrap();assert!(plan.blocks.iter().any(|(id,_)|id=="card/meta"));assert!(!plan.blocks.iter().any(|(id,_)|id.starts_with("file:")));
    f.body("card/meta");let view=f.cache.preview("chat",Some(&visible)).unwrap().unwrap();assert_eq!(view.events[0].attachment.as_ref().unwrap().caption.as_deref(),Some(caption.as_str()));
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
        assert!(tau_block_store::cached_content(&db, "chat", id).unwrap().is_empty());
        assert!(tau_block_store::header(&db, "chat", &format!("file:{id}")).unwrap().is_none());
    }
}

#[test]
fn authored_message_has_one_model_identity_across_receipt_queue_and_history() {
    use crate::feed::{Feed, MessageBody, MessageId};
    let mut f = Fixture::new();
    let text = "keep MY text café 😀, without **interpreting it**";
    let mut local = local_prompt("original-request", text);
    let original = local.pending[0].request.clone();
    let mut feed = Feed::default();
    let id = MessageId::Request(original.id.clone());
    let check = |feed: &Feed, local: &LocalChat| {
        assert_eq!(feed.order, [id.clone()]);
        assert_eq!(feed.messages[&id].text(feed, local), text);
        assert_eq!(id.key("chat"), "message:chat:original-request");
    };
    feed.reconcile(&local);
    check(&feed, &local);
    // Accepted intent survives restart without executing or fabricating delivery.
    local = serde_json::from_slice(&serde_json::to_vec(&local).unwrap()).unwrap();
    assert_eq!(serde_json::to_value(&local.pending[0].request).unwrap(), serde_json::to_value(&original).unwrap());
    f.put(QUEUE, None, i64::MAX as u64, BlockKind::Queue, json!({}), &serde_json::to_vec(&QueueState::native()).unwrap());
    f.put("queued:original-request", Some(QUEUE), 0, BlockKind::Text,
        json!({"request":{"requestId":"original-request","revision":0,"kind":"steer","text":"","images":0}}), text.as_bytes());
    f.page(None, None); f.page(Some(QUEUE), None); f.body(QUEUE);
    feed.native_view(f.cache.snapshot("chat").unwrap().unwrap()).unwrap();
    feed.reconcile(&local); check(&feed, &local);
    assert!(matches!(feed.messages[&id].body, MessageBody::Local(_)));
    f.put("canonical", None, 1, BlockKind::Text, user_body("canonical", "original-request", text), text.as_bytes());
    f.page(None, None);
    feed.native_view(f.cache.snapshot("chat").unwrap().unwrap()).unwrap();
    feed.reconcile(&local); check(&feed, &local);
    assert_eq!(feed.messages[&id].event.as_deref(), Some("canonical"));
    assert_eq!(local.pending.len(), 1, "header overlap is not retirement evidence");
    f.body("canonical");
    let delivered = feed.native_view(f.cache.snapshot("chat").unwrap().unwrap()).unwrap();
    local.reconcile_complete(&feed.queue, &delivered, &feed.incomplete);
    feed.reconcile(&local); check(&feed, &local);
    assert!(local.pending.is_empty());
    assert!(matches!(feed.messages[&id].body, MessageBody::Remote(_)));
}

pub(crate) fn local_prompt(id: &str, text: &str) -> LocalChat {
    use crate::store::{Delivery, Pending};
    LocalChat { pending: vec![Pending { request: tau_net::ClientRequest { id: id.into(),
        command: tau_net::ClientCommand::Prompt { session_id: "chat".into(), text: text.into(), model: None, create: None } },
        text: text.into(), files: vec![], status: Delivery::Accepted, started_at_ms: None, detail: None }], ..Default::default() }
}
pub(crate) fn user_body(id: &str, request: &str, text: &str) -> serde_json::Value {
    let mut meta = event(id, 1, "text");
    meta["event"]["role"] = json!("user");
    meta["event"]["origin"] = json!({"requestId":request});
    meta["bodyHash"] = json!(blake3::hash(text.as_bytes()).to_hex().to_string());
    meta
}

#[test]
fn local_body_reuse_survives_queue_to_history_and_restart_without_downloading_input() {
    let mut f = Fixture::new();
    let text = "my own input café 😀\n".repeat(20000); // More than the render preview budget.
    let mut local = local_prompt("request", &text);
    f.cache.remember_local("chat", &local, &f.lineage).unwrap();
    f.put(QUEUE, None, 100, BlockKind::Queue, json!({}), &serde_json::to_vec(&QueueState::native()).unwrap());
    f.put("queued:request", Some(QUEUE), 0, BlockKind::Text,
        json!({"request":{"requestId":"request","revision":0,"kind":"steer","text":"","images":0},
            "bodyHash":blake3::hash(text.as_bytes()).to_hex().to_string()}), text.as_bytes());
    f.page(None,None); f.body(QUEUE); f.page(Some(QUEUE),None);
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    assert_eq!(view.queue.requests[0].text, text);
    assert!(!view.incomplete.contains("queued:request"));
    local.reconcile_complete(&view.queue, &view.delivered, &view.incomplete);
    assert!(local.pending.is_empty(), "the queue has a complete confirmed copy now");

    // The queue can disappear before the history directory arrives. This must
    // not lose the only reusable bytes or require downloading our own input.
    let tx = f.source.transaction().unwrap();
    tau_block_store::remove(&tx,"chat","queued:request").unwrap(); tx.commit().unwrap();
    f.page(Some(QUEUE),None);
    f.cache = Cache::open(&f._root.path().join("cache.db")).unwrap();
    f.cache.configure(&f.lineage).unwrap();
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request",&text),text.as_bytes());
    f.page(None,None); // Headers only, deliberately no f.body("saved").
    assert_eq!(f.cache.block_request("chat","saved").unwrap().offset, text.len() as u64);
    assert_eq!(f.cache.copy_ready("chat", &["saved".into()]).unwrap().unwrap(), text);
    let plan = f.cache.plan("chat", &local, &[]).unwrap();
    assert!(plan.blocks.iter().all(|(id,head)| id != "saved" || head.is_some_and(|(_,length,sealed,stored)| sealed && length == stored)),
        "a complete sealed body must not start a content stream");
    let view = f.cache.preview("chat",None).unwrap().unwrap();
    assert!(view.incomplete.contains("saved"), "the large message uses a bounded display preview");
    assert_eq!(view.delivered, ["request"], "preview truncation is not missing canonical bytes");
    assert!(!view.events[0].text.starts_with("Loading"));
}

#[test]
fn header_first_reuse_requires_matching_scope_request_digest_and_source() {
    let mut f = Fixture::new();
    let local = local_prompt("request", "same sized text");
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request","same sized text"),b"same sized text");
    f.page(None,None);
    assert!(f.cache.snapshot("chat").unwrap().unwrap().delivered.is_empty());
    f.cache.remember_local("other-chat", &local, &f.lineage).unwrap();
    f.cache.remember_local("chat", &local, "other-source").unwrap();
    f.cache.remember_local("chat", &local_prompt("other-request", "same sized text"), &f.lineage).unwrap();
    f.cache.remember_local("chat", &local_prompt("request", "evil sized text"), &f.lineage).unwrap();
    assert_eq!(f.cache.block_request("chat","saved").unwrap().offset, 0);
    f.cache.remember_local("chat", &local, &f.lineage).unwrap();
    assert_eq!(f.cache.block_request("chat","saved").unwrap().offset, 15);
    assert_eq!(f.cache.snapshot("chat").unwrap().unwrap().delivered, ["request"]);
    f.cache.configure("restored-source").unwrap();
    assert_eq!(f.cache.db.lock().unwrap().query_row("SELECT count(*) FROM local_echoes",[],|r|r.get::<_,u64>(0)).unwrap(), 0);
}

#[test]
fn changed_or_legacy_body_keeps_authored_copy_until_complete_canonical_replacement() {
    let mut f = Fixture::new();
    let mut local = local_prompt("request", "authored text");
    f.cache.remember_local("chat", &local, &f.lineage).unwrap();
    // Same identity, different canonical text: a digest mismatch is not a hit.
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request","server changed text"),b"server changed text");
    f.page(None,None);
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    assert!(view.delivered.is_empty());
    local.reconcile_complete(&view.queue,&view.delivered,&view.incomplete);
    assert_eq!(local.pending[0].text,"authored text");
    f.body("saved");
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    assert_eq!(view.events[0].text,"server changed text");
    local.reconcile_complete(&view.queue,&view.delivered,&view.incomplete);
    assert!(local.pending.is_empty());

    let mut meta = user_body("legacy","legacy-request","authored text");
    meta.as_object_mut().unwrap().remove("bodyHash");
    f.put("legacy",None,2,BlockKind::Text,meta,b"authored text");
    f.cache.remember_local("chat",&local_prompt("legacy-request","authored text"),&f.lineage).unwrap();
    f.page(None,None);
    assert_eq!(f.cache.block_request("chat","legacy").unwrap().offset,0, "IDs alone are not byte verification");
    f.body("legacy");
    assert_eq!(f.cache.block_request("chat","legacy").unwrap().offset,13);
}

#[test]
fn accepted_queue_header_does_not_retire_local_input_before_its_body_arrives() {
    let mut f = Fixture::new();
    let mut local = local_prompt("request","authored text");
    f.put(QUEUE,None,100,BlockKind::Queue,json!({}),&serde_json::to_vec(&QueueState::native()).unwrap());
    f.put("queued:request",Some(QUEUE),0,BlockKind::Text,
        json!({"request":{"requestId":"request","revision":0,"kind":"steer","text":"","images":0}}),b"authored text");
    f.page(None,None); f.body(QUEUE); f.page(Some(QUEUE),None);
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    local.reconcile_complete(&view.queue,&view.delivered,&view.incomplete);
    assert_eq!(local.pending.len(),1);
    f.body("queued:request");
    let view = f.cache.snapshot("chat").unwrap().unwrap();
    local.reconcile_complete(&view.queue,&view.delivered,&view.incomplete);
    assert!(local.pending.is_empty());
}



#[test]
fn queue_removal_before_history_header_keeps_a_display_copy_until_root_catches_up() {
    for (consumed, complete) in [(true,true),(true,false),(false,true)] {
        let mut f=Fixture::new();let text="I should never disappear during the queue handoff";
        let mut local=local_prompt("request",text);
        f.cache.remember_local("chat",&local,&f.lineage).unwrap();
        f.put(QUEUE,None,100,BlockKind::Queue,json!({}),&serde_json::to_vec(&QueueState::native()).unwrap());
        let mut meta=json!({"request":{"requestId":"request","revision":0,"kind":"steer","text":"","images":0}});
        if complete {meta["bodyHash"]=json!(blake3::hash(text.as_bytes()).to_hex().to_string());}
        f.put("queued:request",Some(QUEUE),0,BlockKind::Text,meta,text.as_bytes());
        f.page(None,None);f.body(QUEUE);f.page(Some(QUEUE),None);
        let mut feed=crate::feed::Feed::default();
        let delivered=feed.native_view(f.cache.changes("chat",None,true).unwrap().unwrap()).unwrap();
        local.reconcile_complete(&feed.queue,&delivered,&feed.incomplete);
        assert_eq!(local.pending.is_empty(),complete);
        // Source commits a move/deletion, but deliver the queue directory first.
        let tx=f.source.transaction().unwrap();tau_block_store::remove(&tx,"chat","queued:request").unwrap();tx.commit().unwrap();
        if consumed {f.put("saved",None,1,BlockKind::Text,user_body("saved","request",text),text.as_bytes());}
        else {f.put(QUEUE,None,100,BlockKind::Queue,json!({"membershipHash":"changed"}),&serde_json::to_vec(&QueueState::native()).unwrap());}
        f.page(Some(QUEUE),None);
        feed.native_view(f.cache.changes("chat",None,false).unwrap().unwrap()).unwrap();
        assert!(feed.events.is_empty());assert_eq!(feed.queue.requests.len(),1);
        assert!(feed.queue_transitions.contains_key("request"));
        assert_eq!(feed.incomplete.contains("queued:request"),!complete);
        local.reconcile_complete(&feed.queue,&[],&feed.incomplete);
        assert_eq!(local.pending.is_empty(),complete,"a retained incomplete row cannot retire authored work");
        // A late, empty queue page must not erase the retained row either.
        f.page(Some(QUEUE),None);
        feed.native_view(f.cache.changes("chat",None,false).unwrap().unwrap()).unwrap();
        assert_eq!(feed.queue.requests.len(),1);
        f.page(None,None);
        let delivered=feed.native_view(f.cache.changes("chat",None,false).unwrap().unwrap()).unwrap();
        local.reconcile_complete(&feed.queue,&delivered,&feed.incomplete);
        assert!(feed.queue.requests.is_empty());assert!(feed.queue_transitions.is_empty());
        if consumed {assert_eq!(feed.event("saved").unwrap().text,text);assert!(local.pending.is_empty());}
        else {assert!(feed.events.is_empty(),"explicit deletion must not leave a ghost message");}
    }
}

#[test]
fn known_input_is_not_retired_before_the_viewport_can_display_its_cached_body() {
    let mut f=Fixture::new();let local=local_prompt("request","own input");
    f.cache.remember_local("chat",&local,&f.lineage).unwrap();
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request","own input"),b"own input");
    f.page(None,None);
    let stale_viewport=BTreeSet::new(); // The last frame still displayed the pending row.
    let view=f.cache.preview("chat",Some(&stale_viewport)).unwrap().unwrap();
    assert!(view.delivered.is_empty(),"keep the authored overlay until its replacement is displayable");
    assert!(view.incomplete.contains("saved"));
    let current_viewport=BTreeSet::from(["saved".into()]);
    let view=f.cache.preview("chat",Some(&current_viewport)).unwrap().unwrap();
    assert_eq!(view.events[0].text,"own input");assert_eq!(view.delivered,["request"]);
}

#[test]
fn body_reuse_is_bounded_disposable_and_does_not_accept_corrupt_cached_candidates() {
    let mut f=Fixture::new();
    for n in 0..257 { f.cache.remember_local("chat",&local_prompt(&format!("request-{n}"),"small input"),&f.lineage).unwrap(); }
    {
        let db=f.cache.db.lock().unwrap();
        assert_eq!(db.query_row("SELECT count(*) FROM local_echoes",[],|r|r.get::<_,u64>(0)).unwrap(),256);
        assert_eq!(db.query_row("SELECT count(*) FROM local_echoes WHERE request='request-0'",[],|r|r.get::<_,u64>(0)).unwrap(),0);
        db.execute("UPDATE local_echoes SET body=?1 WHERE request='request-256'",[b"WRONG input".as_slice()]).unwrap();
    }
    f.put("saved",None,1,BlockKind::Text,user_body("saved","request-256","small input"),b"small input");
    f.page(None,None);
    assert_eq!(f.cache.block_request("chat","saved").unwrap().offset,0);
    f.body("saved");assert_eq!(f.cache.block_request("chat","saved").unwrap().offset,11);
    f.cache.clear().unwrap();
    assert_eq!(f.cache.db.lock().unwrap().query_row("SELECT count(*) FROM local_echoes",[],|r|r.get::<_,u64>(0)).unwrap(),0);
}



#[test]
fn warm_scrollback_survives_viewport_and_history_changes_without_stale_replacements() {
    let mut f = Fixture::new();
    for n in 0..100 {
        let id = format!("e{n:03}");
        f.put(&id,None,n,BlockKind::Text,event(&id,n,"text"),format!("body {n}").as_bytes());
    }
    f.page(None,None);
    let mut feed = crate::feed::Feed::default();
    feed.native_view(f.cache.changes("chat",None,true).unwrap().unwrap()).unwrap();
    for n in 68..100 {
        let id = format!("e{n:03}"); f.body(&id);
        let visible = BTreeSet::from([id]);
        let view = f.cache.changes_retaining("chat",Some(&visible),&feed.retained_roots(),false).unwrap().unwrap();
        feed.native_view(view).unwrap();
    }
    // A page prepends metadata, not grounds to replace the screen just read
    // with Loading rows. Their bytes already reside on disk.
    let before = f.cache.history_cursor("chat").unwrap().unwrap();
    f.page(None,Some(before));
    feed.native_view(f.cache.changes_retaining("chat",Some(&BTreeSet::from(["e067".into()])),&feed.retained_roots(),false).unwrap().unwrap()).unwrap();
    for n in 68..100 { assert_eq!(feed.event(&format!("e{n:03}")).unwrap().text,format!("body {n}")); }
    for n in 36..68 {
        let id = format!("e{n:03}");f.body(&id);
        feed.native_view(f.cache.changes_retaining("chat",Some(&BTreeSet::from([id])),&feed.retained_roots(),false).unwrap().unwrap()).unwrap();
    }
    assert_eq!(feed.retained_roots().len(),64,"small messages aren't evicted after just 32 groups");
    assert_eq!(feed.event("e099").unwrap().text,"body 99");
    // A same-ID replacement must invalidate the old bytes even offscreen.
    f.put("e099",None,99,BlockKind::Text,event("e099",99,"text"),b"replacement");
    f.page(None,None);
    feed.native_view(f.cache.changes_retaining("chat",Some(&BTreeSet::from(["e036".into()])),&feed.retained_roots(),false).unwrap().unwrap()).unwrap();
    assert!(feed.event("e099").unwrap().text.is_empty());assert!(feed.bodies["e099"].missing());
    // A full authoritative reset still removes ghost off-window rows.
    f.cache.clear().unwrap();f.page(None,None);
    feed.native_view(f.cache.changes_retaining("chat",None,&feed.retained_roots(),true).unwrap().unwrap()).unwrap();
    assert!(feed.event("e036").is_none());
}

#[test]
fn disk_reads_refresh_eviction_recency_and_saved_anchors_hydrate_offline() {
    let mut f = Fixture::new();
    for n in 0..70 {
        let id = format!("e{n:03}"); f.put(&id,None,n,BlockKind::Text,event(&id,n,"text"),format!("body {n:02}").as_bytes());
    }
    f.page(None,None);
    while let Some(before) = f.cache.history_cursor("chat").unwrap() { f.page(None,Some(before)); }
    f.body("e001"); f.body("e002");
    f.cache = Cache::open(&f._root.path().join("cache.db")).unwrap();
    let mut local = LocalChat::default(); local.position.follow = false;
    // Persisted row identities must all resume the same native root, including
    // the direct tool/thinking rows introduced by retained transcript ownership.
    for key in ["chat/e001", "chat/details:e001", "chat/tool:e001", "chat/thinking:e001"] {
        local.position.key = Some(key.into());
        let visible = f.cache.resume_viewport("chat", &local).unwrap().expect(key);
        assert!(visible.contains("e001")); assert!(!visible.contains("e069"));
        assert_eq!(f.cache.preview("chat",Some(&visible)).unwrap().unwrap().events.iter().find(|e|e.id=="e001").unwrap().text,"body 01");
    }
    f.cache.preview("chat",Some(&BTreeSet::from(["e001".into()]))).unwrap();
    f.body("e003");
    let mut db = f.cache.db.lock().unwrap();let tx=db.transaction().unwrap();
    tau_block_store::enforce_cache_budget(&tx,"chat","e003",14).unwrap();tx.commit().unwrap();
    assert_eq!(tau_block_store::cached_content(&db,"chat","e001").unwrap(),b"body 01","read recency, not last download, protects revisited text");
    assert!(tau_block_store::cached_content(&db,"chat","e002").unwrap().is_empty());
    assert_eq!(tau_block_store::cached_content(&db,"chat","e003").unwrap(),b"body 03");
}

#[test]
fn foreground_projection_precedes_retained_scrollback_and_stays_byte_bounded() {
    let mut f = Fixture::new();
    let bytes = vec![b'x';256*1024];
    for n in 0..40 {
        let id=format!("e{n:03}"); f.put(&id,None,n,BlockKind::Text,event(&id,n,"text"),&bytes);
    }
    f.page(None,None);while let Some(before)=f.cache.history_cursor("chat").unwrap() {f.page(None,Some(before));}
    for n in 0..40 {f.body(&format!("e{n:03}"));}
    let retained=(0..32).map(|n|format!("e{n:03}")).collect();
    let visible=BTreeSet::from(["e039".into()]);
    let view=f.cache.changes_retaining("chat",Some(&visible),&retained,true).unwrap().unwrap();
    assert_eq!(view.events.iter().find(|e|e.id=="e039").unwrap().text.len(),bytes.len());
    assert!(view.previews.iter().map(|(_,_,size)|size).sum::<usize>()<=8*1024*1024);
    let mut feed=crate::feed::Feed::default();feed.native_view(view).unwrap();
    assert_eq!(feed.event("e039").unwrap().text.len(),bytes.len());
}

#[test]
fn background_plans_only_fetch_a_bounded_text_tail_and_queue_not_hidden_details_or_files() {
    let mut f=Fixture::new();
    for n in 0..40 {let id=format!("e{n:03}");f.put(&id,None,n,BlockKind::Text,event(&id,n,"text"),b"answer");}
    f.put("thinking",None,40,BlockKind::Thinking,event("thinking",40,"thinking"),b"hidden thought");
    f.put("tool",None,41,BlockKind::Tool,event("tool",41,"tool"),b"");
    f.put("tool/input",Some("tool"),0,BlockKind::Code,json!({"inputFor":"tool"}),b"hidden input");
    f.put("file:unread",Some("tool"),1,BlockKind::File,json!({"attachment":{}}),b"unread file");
    f.page(None,None);f.page(Some("tool"),None);
    let plan=f.cache.plan_background("chat").unwrap();
    assert!(plan.background);assert!(plan.foreground.is_empty());
    assert_eq!(plan.parents,BTreeSet::from([None,Some(QUEUE.into())]));
    let ids=plan.blocks.iter().map(|(id,_)|id.as_str()).collect::<BTreeSet<_>>();
    assert_eq!(ids.len(),9);assert!(ids.contains("e039"));assert!(ids.contains("e032"));
    for id in ["e031","thinking","tool/input","file:unread"] {assert!(!ids.contains(id));}
    assert!(plan.older.is_empty(),"background work doesn't walk root history");
    let visible=BTreeSet::from(["e020".into()]);
    let active=f.cache.plan_active("chat",&LocalChat::default(),&[],Some(&visible)).unwrap();
    assert!(active.blocks.iter().any(|(id,_)|id=="e020"));
    assert!(active.blocks.iter().any(|(id,_)|id=="e039"),"the latest reply reaches disk even while reading old scrollback");
    assert!(active.foreground.contains("e020"));assert!(!active.foreground.contains("e039"));
    assert_eq!(active.blocks.len(),active.blocks.iter().map(|(id,_)|id).collect::<BTreeSet<_>>().len(),"overlapping interests are deduplicated");
}

#[test]
fn controller_keeps_recent_views_warm_and_reopens_evicted_scrollback_from_disk() {
    use crate::{controller::Controller,store::Store};
    use tau_net::{ServerMessage,SessionSummary};
    let mut f=Fixture::new();
    let mut c=Controller::new(Store::open(f._root.path().join("client")).unwrap(),Arc::new(||{})).unwrap();
    f.cache=c.store.block_cache(&c.identity).unwrap();f.cache.configure(&f.lineage).unwrap();
    for n in 0..80 {let id=format!("e{n:03}");f.put(&id,None,n,BlockKind::Text,event(&id,n,"text"),format!("body {n}").as_bytes());}
    f.page(None,None);while let Some(before)=f.cache.history_cursor("chat").unwrap() {f.page(None,Some(before));}
    f.body("e001");f.body("e002");
    let sessions=["chat","b","c","d","e"].into_iter().map(|id| serde_json::from_value::<SessionSummary>(json!({
        "id":id,"title":id,"starter":false,"status":"idle","createdAtMs":1,"updatedAtMs":1
    })).unwrap()).collect();
    c.message(ServerMessage::Sessions {sessions}).unwrap();c.select("chat").unwrap();
    c.viewport("chat",BTreeSet::from(["e001".into()]));
    c.viewport("chat",BTreeSet::from(["e002".into()]));
    assert_eq!(c.chats["chat"].feed.event("e001").unwrap().text,"body 1","leaving the viewport isn't eviction");
    c.chats.get_mut("chat").unwrap().local.position=crate::store::Position {follow:false,key:Some("chat/e001".into()),offset:5.};
    c.draft("keep my draft".into()).unwrap();
    c.select("b").unwrap();assert_eq!(c.chats["chat"].feed.event("e001").unwrap().text,"body 1");
    c.select("chat").unwrap();assert_eq!(c.selected().unwrap().feed.event("e001").unwrap().text,"body 1");
    for id in ["b","c","d","e"] {c.select(id).unwrap();}
    assert_eq!(c.chats["chat"].feed.event("e001").unwrap().text,"body 1","small views are not evicted after an arbitrary number of chats");
    c.trim_chat_views(0); // Simulate memory pressure, not a chat-count limit.
    assert!(c.chats["chat"].feed.events.is_empty(),"cold views leave memory while authored work survives");
    assert_eq!(c.chats["chat"].local.draft,"keep my draft");
    c.select("chat").unwrap();
    assert_eq!(c.selected().unwrap().feed.event("e001").unwrap().text,"body 1","disk hydration targets the saved scroll anchor, not only the tail");
    assert_eq!(c.selected().unwrap().local.position.offset,5.);
    assert!(c.epoch.is_none(),"all return visits worked offline");
}

#[test]
fn busy_recency_never_gates_verified_reads_or_weakens_content_writes() {
    let mut f = Fixture::new();
    f.put("text", None, 1, BlockKind::Text, event("text", 1, "text"), b"verified body");
    f.page(None, None); f.body("text");
    let clock = || f.cache.db.lock().unwrap().query_row("SELECT clock FROM block_usage", [], |r| r.get::<_,u64>(0)).unwrap();
    let before = clock();
    let writer = Connection::open(f._root.path().join("cache.db")).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();
    let view = f.cache.preview("chat", None).unwrap().unwrap();
    assert_eq!(view.events[0].text, "verified body");
    let local = LocalChat { pending: vec![crate::store::Pending {
        request: tau_net::ClientRequest { id: "uncertain".into(), command: tau_net::ClientCommand::Prompt { session_id: "chat".into(), text: "keep locally".into(), model: None, create: None } },
        started_at_ms: None, text: "keep locally".into(), files: vec![],
        status: crate::store::Delivery::Unconfirmed, detail: None,
    }], ..Default::default() };
    f.cache.remember_local("chat", &local, &f.lineage).unwrap();
    assert_eq!(f.cache.db.lock().unwrap().query_row("SELECT count(*) FROM local_echoes", [], |r| r.get::<_,u32>(0)).unwrap(), 0);
    assert_eq!(clock(), before, "busy optional recency was skipped, not a required content write");
    assert_eq!(f.cache.db.lock().unwrap().query_row("PRAGMA busy_timeout", [], |r| r.get::<_,u32>(0)).unwrap(), 5000);
    writer.execute_batch("ROLLBACK").unwrap();
    f.cache.preview("chat", None).unwrap();
    assert!(clock() > before, "uncontended visits still protect bodies from eviction");
    f.cache.remember_local("chat", &local, &f.lineage).unwrap();
    assert_eq!(f.cache.db.lock().unwrap().query_row("SELECT count(*) FROM local_echoes", [], |r| r.get::<_,u32>(0)).unwrap(), 1, "local body reuse resumes when uncontended");
    writer.execute_batch("CREATE TRIGGER fail_touch BEFORE UPDATE ON block_usage BEGIN SELECT RAISE(ABORT,'recency fault'); END").unwrap();
    let error = f.cache.preview("chat", None).err().expect("non-contention failures must not be hidden");
    assert!(format!("{error:#}").contains("recency fault"));
    assert_eq!(tau_block_store::cached_content(&writer, "chat", "text").unwrap(), b"verified body");
}

#[test]
fn live_projection_does_not_add_a_ui_thread_recency_write_per_chunk() {
    let mut f=Fixture::new();
    f.put("text",None,1,BlockKind::Text,event("text",1,"text"),b"body");f.page(None,None);f.body("text");
    f.cache.changes("chat",None,true).unwrap();
    let clock=||f.cache.db.lock().unwrap().query_row("SELECT clock FROM block_usage",[],|r|r.get::<_,u64>(0)).unwrap();
    let before=clock();
    f.cache.changed("chat","text",false);
    f.cache.changes("chat",Some(&BTreeSet::from(["text".into()])),false).unwrap();
    assert_eq!(clock(),before,"network-driven projection already has download recency");
    f.cache.viewport_changed("chat",["text".into()].into_iter());
    f.cache.changes("chat",Some(&BTreeSet::from(["text".into()])),false).unwrap();
    assert!(clock()>before,"an actual visit refreshes eviction recency");
}



#[test]
fn prefetched_bodies_do_not_thrash_after_eviction_but_replacements_and_viewing_still_fetch() {
    let mut f=Fixture::new();
    f.put("text",None,1,BlockKind::Text,event("text",1,"text"),b"body");f.page(None,None);f.body("text");
    f.cache.plan_background("chat").unwrap();
    {let db=f.cache.db.lock().unwrap();db.execute("DELETE FROM block_parts WHERE scope='chat' AND id='text'",[]).unwrap();}
    assert!(!f.cache.plan_background("chat").unwrap().blocks.iter().any(|(id,_)|id=="text"),"idle background work must not refill evicted bytes forever");
    assert!(f.cache.plan_active("chat",&LocalChat::default(),&[],Some(&BTreeSet::from(["text".into()]))).unwrap().blocks.iter().any(|(id,_)|id=="text"),"actual viewing bypasses the prefetch watermark");
    f.put("text",None,1,BlockKind::Text,event("text",1,"text"),b"replacement");f.page(None,None);
    assert!(f.cache.plan_background("chat").unwrap().blocks.iter().any(|(id,_)|id=="text"));
    f.body("text");f.cache.plan_background("chat").unwrap();
    f.cache.clear().unwrap();f.page(None,None);
    assert!(f.cache.plan_background("chat").unwrap().blocks.iter().any(|(id,_)|id=="text"),"explicit cache clearing resets prefetch completion");
}
