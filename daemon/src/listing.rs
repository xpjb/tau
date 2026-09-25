//! Keyset-paged metadata. Membership/name/model/topic changes fence a traversal;
//! live activity does not restart it. Runtime status has its own monotonic stamp.
use anyhow::Result;
use rusqlite::params;
use tau_protocol::{Project,ServerMessage,SessionSummary,SessionStatus};
use crate::{manager::AgentManager,state::StoredSession};
impl AgentManager {
    pub(crate) async fn list_page(&self,catalog_id:String,projects:bool,after:Option<String>,revision:u64)->Result<ServerMessage> {
        let (revision,after,rows)=self.inner.state.access(move |db| {
            let current:u64=db.query_row("SELECT revision FROM catalogue_clock",[],|r|r.get(0))?;
            let after=if current==revision {after} else {None};
            let rows=if projects {
                db.prepare("SELECT id,json_object('id',id,'name',name,'prompt',prompt,'revision',revision) FROM projects WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT 9")?
                    .query_map([&after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
            } else {
                // Never copy a captured project prompt (up to 256 KiB per chat)
                // into a list query, nor load history or construct runtimes.
                db.prepare("SELECT id,json_remove(data,'$.project_prompt') FROM sessions WHERE ?1 IS NULL OR id>?1 ORDER BY id LIMIT 65")?
                    .query_map(params![after],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?)))?.collect::<rusqlite::Result<Vec<_>>>()?
            };
            Ok((current,after,rows))
        }).await?;
        let limit=if projects {8} else {64};
        let next=(rows.len()>limit).then(||rows[limit-1].0.clone());
        if projects {
            let projects=rows.into_iter().take(limit).map(|(_,data)|serde_json::from_str::<Project>(&data)).collect::<serde_json::Result<_>>()?;
            Ok(ServerMessage::ProjectPage {catalog_id,revision,after,next,projects})
        } else {
            let settings=self.inner.settings.get();let runtimes=self.inner.runtimes.lock().await;
            let cold_revision=self.inner.state_clock.load(std::sync::atomic::Ordering::Acquire);
            let mut providers=std::collections::HashSet::new();
            let mut sessions=Vec::new();let mut states=std::collections::BTreeMap::new();
            for (id,data) in rows.into_iter().take(limit) {
                let stored:StoredSession=serde_json::from_str(&data)?;
                if providers.len()<8 && providers.insert(stored.model.provider.clone()) {self.schedule_catalog(&stored.model.provider);}
                let state=runtimes.get(&id).map(|runtime|runtime.snapshot());
                states.insert(id.clone(),state.as_ref().map_or(cold_revision,|s|s.revision));
                sessions.push(SessionSummary {id,title:stored.title,project_id:stored.project_id,starter:stored.starter,parent_id:stored.parent_id,model:Some(stored.model.clone()),
                    status:state.as_ref().map(|s|s.status).unwrap_or(SessionStatus::Sleeping),detail:state.as_ref().and_then(|s|s.detail.clone()),
                    context_usage:state.as_ref().and_then(|s|s.context_usage.clone()).or_else(||self.context_usage(&settings,&stored.model,stored.tokens)),created_at_ms:stored.created_at_ms,updated_at_ms:stored.updated_at_ms});
            }
            Ok(ServerMessage::SessionPage {catalog_id,revision,after,next,sessions,states})
        }
    }
}
