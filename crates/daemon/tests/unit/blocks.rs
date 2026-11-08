use super::*;
use crate::transcript::EventProjection;
#[test]
fn user_and_queue_headers_certify_exact_text_without_embedding_it() {
    let mut db = Connection::open_in_memory().unwrap();
    db.execute_batch("PRAGMA foreign_keys=ON").unwrap(); tau_block_store::initialize(&db).unwrap();
    let text = "authored café 😀\n".repeat(3000);
    let raw=json!({"type":"message","id":"entry","origin":{"requestId":"request"},"message":{"role":"user","content":[{"type":"text","text":text}]}});
    let value=Event::from_entry(&raw,false).unwrap().remove(0);
    let tx=db.transaction().unwrap(); event(&tx,"chat",&value).unwrap();
    let mut state=QueueState::native();state.requests.push(tau_net::QueuedRequest {request_id:"request".into(),revision:0,text:text.clone(),timestamp_ms:None});
    queue(&tx,"chat",&state).unwrap();tx.commit().unwrap();
    for id in [&value.id,"queued:request"] {
        let h=tau_block_store::header(&db,"chat",id).unwrap().unwrap();
        assert_eq!(h.meta["bodyHash"],blake3::hash(text.as_bytes()).to_hex().as_str());
        assert_eq!(h.length,text.len() as u64);assert!(h.sealed);
        assert!(serde_json::to_vec(&h).unwrap().len()<tau_net::blocks::MAX_BLOCK_HEADER_BYTES);
        assert!(!h.meta.to_string().contains("authored café"));
    }
    let before=tau_block_store::header(&db,"chat",QUEUE).unwrap().unwrap();
    state.requests.clear();
    let tx=db.transaction().unwrap();queue(&tx,"chat",&state).unwrap();tx.commit().unwrap();
    let after=tau_block_store::header(&db,"chat",QUEUE).unwrap().unwrap();
    assert!(after.revision>before.revision,"even deletion-only changes advance the root directory");
    assert_eq!(after.version,before.version,"unchanged queue-state bytes are not downloaded again");
}

#[test]
fn oversized_metadata_is_preserved_as_a_referenced_body() {
    let mut db=Connection::open_in_memory().unwrap();db.execute_batch("PRAGMA foreign_keys=ON").unwrap();tau_block_store::initialize(&db).unwrap();
    let raw=json!({"type":"message","id":"entry","message":{"role":"assistant","content":[{"type":"text","text":"answer"}],"errorMessage":"\0🦀".repeat(5000)}});
    let mut value=crate::transcript::Event::from_entry(&raw,false).unwrap().remove(0);value.order=1;
    let tx=db.transaction().unwrap();event(&tx,"chat",&value).unwrap();tx.commit().unwrap();
    let h=tau_block_store::header(&db,"chat",&value.id).unwrap().unwrap();assert!(serde_json::to_vec(&h).unwrap().len()<=tau_net::blocks::MAX_BLOCK_HEADER_BYTES);
    let id=h.meta["fullEvent"]["id"].as_str().unwrap();let bytes=tau_block_store::cached_content(&db,"chat",id).unwrap();
    let restored:Event=serde_json::from_slice(&bytes).unwrap();assert_eq!(restored.error_message,value.error_message);
    assert_eq!(tau_block_store::cached_content(&db,"chat",&value.id).unwrap(),b"answer");
}
#[test]
fn tool_projection_keeps_raw_streamed_input_and_seals_without_replacement() {
    let mut db=Connection::open_in_memory().unwrap();db.execute_batch("PRAGMA foreign_keys=ON").unwrap();tau_block_store::initialize(&db).unwrap();
    let raw=format!("{{\"command\":\"{}\"}}","x".repeat(40000));
    let stream="stable-stream";
    let project=|raw:&str,saved:bool| {
        let message=json!({"role":"assistant","content":[{"type":"toolCall","id":"call","name":"bash","partialArguments":raw,"arguments":serde_json::from_str::<Value>(raw).unwrap_or(Value::Null)}]});
        let entry=if saved {json!({"type":"message","id":"saved-entry","origin":{"streamId":stream},"message":message})}
            else {json!({"streamId":stream,"message":message})};
        crate::transcript::Event::from_entry(&entry,!saved).unwrap().remove(0)
    };
    for prefix in [100,10000,raw.len()] {let tx=db.transaction().unwrap();event(&tx,"chat",&project(&raw[..prefix],false)).unwrap();tx.commit().unwrap();}
    let finished=project(&raw,true);let input=input_id(&finished.id);let before=tau_block_store::header(&db,"chat",&input).unwrap().unwrap();
    let tx=db.transaction().unwrap();event(&tx,"chat",&finished).unwrap();tx.commit().unwrap();
    let range=tau_block_store::read(&db,&tau_net::blocks::BlockRequest {scope:"chat".into(),id:input,version:before.version,offset:before.length,follow:false}).unwrap().unwrap();
    assert!(range.bytes.is_empty());assert!(range.header.sealed);assert_eq!(range.header.version,before.version);
    let card=tau_block_store::header(&db,"chat",&finished.id).unwrap().unwrap();assert_eq!(card.length,0);
    assert!(serde_json::to_vec(&card).unwrap().len()<2048);assert_eq!(tau_block_store::children(&db,"chat",Some(&card.id)).unwrap().len(),1);
}
#[tokio::test]
async fn published_tool_contract_handles_reused_ids_orphans_overflow_and_interruption() {
    let root=tempfile::tempdir().unwrap();let state=StateStore::load(root.path().join("state.db")).await.unwrap();
    state.access(|db| {
        let tx=db.transaction()?;
        let provider="provider-reused".repeat(180);
        let mut ids=std::collections::HashMap::new();
        for (order,(id,call,error,parent)) in [
            ("orphan",false,false,None), ("a",true,false,None), ("one",false,false,Some("a")),
            ("b",true,false,None), ("two",false,false,Some("b")), ("error",false,true,Some("b")),
            ("unfinished",true,false,None),
        ].into_iter().enumerate() {
            let message=if call {json!({"role":"assistant","content":[{"type":"toolCall","id":provider,"name":"bash","arguments":{"command":"true"}}]})}
                else {json!({"role":"toolResult","toolCallId":provider,"toolName":"bash","isError":error,"content":[{"type":"text","text":id}]})};
            let mut value=Event::from_entry(&json!({"type":"message","id":id,"message":message}),false)?.remove(0);
            value.order=order as u64;event(&tx,"chat",&value)?;ids.insert(id,value.id.clone());
            let h=tau_block_store::header(&tx,"chat",&value.id)?.unwrap();
            assert_eq!(h.parent.as_ref(),parent.map(|p|&ids[p]));
            let metadata=tau_block_store::cached_content(&tx,"chat",h.meta["fullEvent"]["id"].as_str().unwrap())?;
            assert_eq!(serde_json::from_slice::<Event>(&metadata)?.tool_call_id.as_deref(),Some(provider.as_str()));
            if call {
                let input=tau_block_store::header(&tx,"chat",&input_id(&h.id))?.unwrap();
                assert_eq!(input.tool_body(),Some(ToolBody::Input));
                assert_eq!(h.tool_state(),Some(ToolState::Running));
            } else {assert_eq!(h.tool_body(),Some(if error {ToolBody::Error} else {ToolBody::Output}));}
        }
        for (id,expected) in [("a",ToolState::Completed),("b",ToolState::Failed)] {
            assert_eq!(tau_block_store::header(&tx,"chat",&ids[id])?.unwrap().tool_state(),Some(expected));
        }
        recover(&tx)?;
        let interrupted=tau_block_store::header(&tx,"chat",&ids["unfinished"])?.unwrap();
        assert_eq!(interrupted.tool_state(),Some(ToolState::Interrupted));
        assert!(interrupted.sealed && tau_block_store::header(&tx,"chat",&input_id(&interrupted.id))?.unwrap().sealed);
        tx.commit()?;Ok(())
    }).await.unwrap();
}
