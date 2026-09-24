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
