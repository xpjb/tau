//! Durable ownership for control mutations which do not already have a native
//! prompt/queue receipt. A reservation owns the complete immutable command, not
//! an upload reference. Interrupted effects are never automatically replayed.
use anyhow::{Result,ensure};
use rusqlite::{OptionalExtension,params};
use tau_protocol::{ClientCommand,ClientRequest,ServerMessage};
use crate::state::StateStore;

impl StateStore {
    pub(crate) async fn reserve_operation(&self,request:&ClientRequest)->Result<Option<ServerMessage>> {
        let id=request.id.clone();let payload=serde_json::to_string(&request.command)?;
        let legacy_create=if let ClientCommand::CreateSession {project_id,keep_session_id}=&request.command {
            Some(serde_json::json!({"keepSessionId":keep_session_id,"projectId":project_id}).to_string())
        } else {None};
        self.access(move |db| {
            let tx=db.transaction()?;
            if let Some((original,response))=tx.query_row("SELECT payload,response FROM operations WHERE id=?1",[&id],|r|Ok((r.get::<_,String>(0)?,r.get::<_,Option<String>>(1)?))).optional()? {
                ensure!(original==payload,"Operation ID was already used for a different command");
                return Ok(Some(if let Some(response)=response {serde_json::from_str(&response)?} else {
                    let mut response=ServerMessage::failure(id,"Operation is already accepted; its outcome is not yet available");
                    if let ServerMessage::Response {uncertain,..}=&mut response {*uncertain=true;}response
                }));
            }
            let legacy=tx.prepare("SELECT data FROM receipts WHERE request_id=?1")?.query_map([&id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for data in legacy {
                let receipt:crate::state::Receipt=serde_json::from_str(&data)?;
                ensure!(receipt.command.as_deref()==Some("create_session") && legacy_create.as_ref()==Some(&receipt.text),"Operation ID was already used for a different intent");
            }
            tx.execute("INSERT INTO operations(id,payload) VALUES(?1,?2)",params![id,payload])?;tx.commit()?;Ok(None)
        }).await
    }
    pub(crate) async fn finish_operation(&self,id:String,response:&ServerMessage)->Result<()> {
        let response=serde_json::to_string(response)?;
        self.access(move |db| {ensure!(db.execute("UPDATE operations SET response=?2 WHERE id=?1 AND response IS NULL",params![id,response])?==1,"Operation ownership changed");Ok(())}).await
    }
    pub(crate) async fn operation_outcome(&self,id:&str)->Result<ServerMessage> {
        ensure!(!id.is_empty() && id.len()<=128,"Invalid operation ID");let id=id.to_owned();
        self.access(move |db| {
            let row=db.query_row("SELECT response FROM operations WHERE id=?1",[&id],|r|r.get::<_,Option<String>>(0)).optional()?;
            let registered=row.is_some();let response=row.flatten().map(|s|serde_json::from_str(&s).map(Box::new)).transpose()?;
            Ok(ServerMessage::Operation {operation_id:id,registered,response})
        }).await
    }
    pub(crate) async fn operation_receipt(&self,id:&str)->Result<Option<tau_protocol::OperationReceipt>> {
        let id=id.to_owned();
        self.access(move |db| {
            let row=db.query_row("SELECT response FROM operations WHERE id=?1",[&id],|r|r.get::<_,Option<String>>(0)).optional()?;
            row.map(|response| {
                let complete=response.is_some();
                let (error,notice)=if let Some(response)=response {
                    match serde_json::from_str::<ServerMessage>(&response)? {ServerMessage::Response {error,notice,..}=>(error,notice),_=>(None,None)}
                } else {(None,None)};
                Ok(tau_protocol::OperationReceipt {id,accepted:true,complete,error,notice})
            }).transpose()
        }).await
    }
    pub(crate) async fn recover_operations(&self)->Result<()> {
        self.access(|db| {
            let tx=db.transaction()?;
            let ids=tx.prepare("SELECT id FROM operations WHERE response IS NULL")?.query_map([],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
            for id in ids {
                let mut response=ServerMessage::failure(id.clone(),"Operation interrupted by daemon restart; reconcile its effects before issuing another ID");
                if let ServerMessage::Response {uncertain,..}=&mut response {*uncertain=true;}
                tx.execute("UPDATE operations SET response=?2 WHERE id=?1",params![id,serde_json::to_string(&response)?])?;
            }
            tx.commit()?;Ok(())
        }).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn lost_generic_response_is_replayed_but_interrupted_effect_is_never_reexecuted() {
        let root=tempfile::tempdir().unwrap();let path=root.path().join("db");let state=StateStore::load(path.clone()).await.unwrap();
        let request=ClientRequest {id:"mutation".into(),command:ClientCommand::RenameSession {session_id:"chat".into(),title:"title".into()}};
        assert!(state.reserve_operation(&request).await.unwrap().is_none());
        let response=ServerMessage::success(request.id.clone(),Some("chat".into()),None);
        state.finish_operation(request.id.clone(),&response).await.unwrap();
        let mut interrupted=request.clone();interrupted.id="interrupted".into();assert!(state.reserve_operation(&interrupted).await.unwrap().is_none());
        drop(state);
        let state=StateStore::load(path).await.unwrap();state.recover_operations().await.unwrap();
        assert!(matches!(state.reserve_operation(&request).await.unwrap(),Some(ServerMessage::Response {ok:true,uncertain:false,..})));
        assert!(matches!(state.reserve_operation(&interrupted).await.unwrap(),Some(ServerMessage::Response {ok:false,uncertain:true,..})));
        let mut conflict=request;conflict.command=ClientCommand::DeleteSession {session_id:"chat".into()};assert!(state.reserve_operation(&conflict).await.is_err());
        assert!(state.operation_receipt("mutation").await.unwrap().unwrap().complete);
    }
}
