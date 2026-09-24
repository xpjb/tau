//! Hard-bounded control frames. Large application descriptors are immutable
//! native blocks. Small durable receipt summaries never wait on their download.
use anyhow::{Result, ensure};
use tau_blocks::*;
use tau_protocol::{ClientCommand, ClientRequest, OperationReceipt, ServerMessage, MAX_CONTROL_BYTES};
use crate::state::StateStore;

impl StateStore {
    pub(crate) async fn resolve_input(&self, request: ClientRequest) -> Result<ClientRequest> {
        let ClientCommand::Input {content} = request.command else {return Ok(request);};
        let bytes = self.access(move |db|tau_blocks::uploads::input(db,&content)).await?;
        let decoded: ClientRequest = serde_json::from_slice(&bytes)?;
        ensure!(decoded.id == request.id,"Input request ID does not match its control descriptor");
        ensure!(!matches!(decoded.command,ClientCommand::Input {..} | ClientCommand::ConnectBlocks {..}),"Invalid nested input");
        Ok(decoded)
    }
    pub(crate) async fn control_frame(&self, message: &ServerMessage) -> Result<String> {
        let bytes = serde_json::to_vec(message)?;
        if bytes.len() <= MAX_CONTROL_BYTES {return Ok(String::from_utf8(bytes)?);}
        ensure!(bytes.len() as u64 <= MAX_BLOCK_BYTES,"Descriptor exceeds its content limit");
        let hash = blake3::hash(&bytes).to_hex().to_string();
        let id = hash.clone();
        let length = bytes.len() as u64;
        let lineage = self.access(move |db| {
            let tx = db.transaction()?;
            let now=std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH)?.as_secs();
            tx.execute("DELETE FROM blocks WHERE scope=?1 AND position<?2",rusqlite::params![CONTROL_SCOPE,now.saturating_sub(24*3600)])?;
            tx.execute("DELETE FROM block_changes WHERE scope=?1 AND id NOT IN (SELECT id FROM blocks WHERE scope=?1)",[CONTROL_SCOPE])?;
            if tau_blocks::header(&tx,CONTROL_SCOPE,&id)?.is_none() {
                let (count,size):(u64,u64)=tx.query_row("SELECT count(*),coalesce(sum(json_extract(header,'$.length')),0) FROM blocks WHERE scope=?1",[CONTROL_SCOPE],|r|Ok((r.get(0)?,r.get(1)?)))?;
                ensure!(count<4096 && size.saturating_add(length)<=256*1024*1024,"Descriptor storage quota is full");
                let h = BlockHeader {id:id.clone(),parent:None,order:now,kind:BlockKind::State,meta:serde_json::json!({"descriptor":true}),version:0,length:0,sealed:true,revision:0};
                tau_blocks::put(&tx,CONTROL_SCOPE,h,&bytes)?;
            }
            // Renew the immutable descriptor's lease without changing bytes.
            tx.execute("UPDATE blocks SET position=?3 WHERE scope=?1 AND id=?2",rusqlite::params![CONTROL_SCOPE,id,now])?;
            let lineage = tau_blocks::cursor(&tx)?.lineage;
            tx.commit()?; Ok(lineage)
        }).await?;
        let short = |s: &Option<String>| s.as_ref().map(|s| {
            let mut n = s.len().min(32);while !s.is_char_boundary(n) {n-=1;}
            if n < s.len() {format!("{}… (full details downloading)",&s[..n])} else {s.clone()}
        });
        let (session_id,reports) = match message {
            ServerMessage::Response {request_id,ok,session_id,error,notice,uncertain,..} if !uncertain => (session_id.clone(),vec![OperationReceipt {id:request_id.clone(),accepted:*ok,complete:true,error:short(error),notice:short(notice)}]),
            ServerMessage::Receipts {session_id,reports} => (Some(session_id.clone()),reports.iter().map(|r|OperationReceipt {id:r.id.clone(),accepted:r.accepted,complete:r.complete,error:short(&r.error),notice:short(&r.notice)}).collect()),
            _ => (None,vec![]),
        };
        let operation_id=if let ServerMessage::Operation {operation_id,registered:true,..}=message {Some(operation_id.clone())} else {None};
        let descriptor = ServerMessage::Data {operation_id,key:message.replication_key().unwrap_or_else(||format!("receipt:{hash}")),content:ContentRef {lineage,scope:CONTROL_SCOPE.into(),id:hash.clone(),length,hash},session_id,reports};
        let encoded = serde_json::to_string(&descriptor)?;
        ensure!(encoded.len() <= MAX_CONTROL_BYTES,"Receipt summary exceeds the control limit");
        Ok(encoded)
    }
}
