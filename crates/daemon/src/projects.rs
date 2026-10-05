//! Durable project metadata and atomic membership/deletion. The manager's project
//! gate orders membership changes against runtime retirement; SQLite commits first.
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
#[cfg(test)] use tau_net::Project;
use tau_net::{DeleteProjectMode, GENERAL_PROJECT_ID, MAX_PROJECT_NAME_CHARS, MAX_PROJECT_PROMPT_CHARS};
use crate::{manager::AgentManager, state::StateStore};

fn validate(name: &str, prompt: &str) -> Result<()> {
    ensure!(!name.trim().is_empty() && name.chars().count() <= MAX_PROJECT_NAME_CHARS
        && !name.chars().any(char::is_control), "Topic name must contain 1–{MAX_PROJECT_NAME_CHARS} characters on one line");
    ensure!(prompt.chars().count() <= MAX_PROJECT_PROMPT_CHARS, "Topic prompt is too long");
    Ok(())
}
impl StateStore {
    #[cfg(test)]
    pub async fn projects(&self) -> Result<Vec<Project>> {
        self.access(|db| {
            let mut query = db.prepare("SELECT id,name,prompt,revision FROM projects ORDER BY id='general' DESC,rowid")?;
            Ok(query.query_map([], |r| Ok(Project { id:r.get(0)?,name:r.get(1)?,prompt:r.get(2)?,revision:r.get(3)? }))?
                .collect::<rusqlite::Result<_>>()?)
        }).await
    }
    pub async fn create_project(&self, id: String, name: String, prompt: String) -> Result<()> {
        validate(&name, &prompt)?;
        uuid::Uuid::parse_str(&id).context("Invalid topic ID")?;
        self.access(move |db| {
            let tx = db.transaction()?;
            ensure!(tx.query_row("SELECT count(*) FROM projects", [], |r| r.get::<_,u64>(0))? < 128, "At most 128 topics");
            tx.execute("INSERT INTO projects(id,name,prompt) VALUES(?1,?2,?3)", params![id,name.trim(),prompt])?;
            tx.commit()?; Ok(())
        }).await
    }
    pub async fn update_project(&self, id: String, revision: u64, name: String, prompt: String) -> Result<()> {
        validate(&name, &prompt)?;
        ensure!(id != GENERAL_PROJECT_ID || name == "General", "General cannot be renamed");
        ensure!(revision < i64::MAX as u64, "Topic revision exhausted");
        self.access(move |db| {
            ensure!(db.execute("UPDATE projects SET name=?3,prompt=?4,revision=revision+1 WHERE id=?1 AND revision=?2",
                params![id,revision,name.trim(),prompt])? == 1, "Topic changed or was deleted; reopen its settings");
            Ok(())
        }).await
    }
    pub async fn project_prompt(&self, session: &str) -> Result<String> {
        let session = session.to_owned();
        self.access(move |db| Ok(db.query_row("SELECT coalesce(json_extract(data,'$.project_prompt'),'') FROM sessions WHERE id=?1",
            [&session], |r| r.get(0)).context("Chat no longer exists")?)).await
    }
    pub async fn move_session(&self, session: String, project: String) -> Result<()> {
        self.access(move |db| {
            let tx = db.transaction()?;
            let prompt: String = tx.query_row("SELECT prompt FROM projects WHERE id=?1", [&project], |r| r.get(0)).context("Unknown topic")?;
            let current: String = tx.query_row("SELECT json_extract(data,'$.project_id') FROM sessions WHERE id=?1", [&session], |r| r.get(0)).context("Unknown chat")?;
            if current == project { return Ok(()); }
            // A destination may already have a reusable new-chat tile. Never
            // remove either chat (or its client-owned draft) to satisfy the index.
            let occupied: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sessions WHERE starter=1 AND json_extract(data,'$.project_id')=?1 AND id!=?2)",
                params![project,session], |r| r.get(0))?;
            ensure!(tx.execute("UPDATE sessions SET starter=CASE WHEN ?3 THEN 0 ELSE starter END,
                data=json_set(data,'$.project_id',?2,'$.project_prompt',?4,'$.starter',json(CASE WHEN ?3 OR starter=0 THEN 'false' ELSE 'true' END)) WHERE id=?1",
                params![session,project,occupied,prompt])? == 1, "Unknown chat");
            tx.commit()?; Ok(())
        }).await
    }
    pub async fn project_sessions(&self, id: &str, revision: u64) -> Result<Vec<String>> {
        ensure!(id != GENERAL_PROJECT_ID, "General cannot be deleted");
        let id = id.to_owned();
        self.access(move |db| {
            let current: Option<u64> = db.query_row("SELECT revision FROM projects WHERE id=?1", [&id], |r| r.get(0)).optional()?;
            ensure!(current == Some(revision), "Topic changed or was deleted; reopen its settings");
            let mut query = db.prepare("SELECT id FROM sessions WHERE json_extract(data,'$.project_id')=?1 ORDER BY id")?;
            Ok(query.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        }).await
    }
    pub async fn delete_project(&self, id: String, revision: u64, mode: DeleteProjectMode) -> Result<()> {
        ensure!(id != GENERAL_PROJECT_ID, "General cannot be deleted");
        self.access(move |db| {
            let tx = db.transaction()?;
            ensure!(tx.execute("DELETE FROM projects WHERE id=?1 AND revision=?2", params![id,revision])? == 1,
                "Topic changed or was deleted; reopen its settings");
            match mode {
                DeleteProjectMode::MoveToGeneral => {
                    tx.execute("UPDATE sessions SET starter=0,data=json_set(data,'$.project_id','general','$.project_prompt',(SELECT prompt FROM projects WHERE id='general'),'$.starter',json('false')) WHERE json_extract(data,'$.project_id')=?1", [&id])?;
                }
                DeleteProjectMode::DeleteChats => {
                    let ids = tx.prepare("SELECT id FROM sessions WHERE json_extract(data,'$.project_id')=?1")?
                        .query_map([&id],|r|r.get::<_,String>(0))?.collect::<rusqlite::Result<Vec<_>>>()?;
                    for scope in ids { tau_block_store::remove_scope(&tx,&scope)?; }
                    tx.execute("DELETE FROM sessions WHERE json_extract(data,'$.project_id')=?1", [&id])?;
                    tx.execute("UPDATE sessions SET data=json_set(data,'$.parent_id',NULL) WHERE json_extract(data,'$.parent_id') IS NOT NULL AND NOT EXISTS
                        (SELECT 1 FROM sessions parent WHERE parent.id=json_extract(sessions.data,'$.parent_id'))", [])?;
                }
            }
            tx.commit()?; Ok(())
        }).await
    }
}
impl AgentManager {
    async fn broadcast_projects(&self) -> Result<()> {
        let _ = self.inner.events.send(tau_net::ServerMessage::ResyncRequired {session_id:None});
        Ok(())
    }
    pub async fn create_project(&self, id: String, name: String, prompt: String) -> Result<()> {
        let _gate = self.inner.projects.lock().await;
        self.inner.state.create_project(id,name,prompt).await?;
        self.broadcast_projects().await
    }
    pub async fn update_project(&self, id: String, revision: u64, name: String, prompt: String) -> Result<()> {
        let _gate = self.inner.projects.lock().await;
        self.inner.state.update_project(id,revision,name,prompt).await?;
        self.broadcast_projects().await
    }
    pub async fn move_session(&self, id: &str, project: String) -> Result<()> {
        let _gate = self.inner.projects.lock().await;
        let runtime = self.runtime(id).await?;
        let _operation = runtime.operation.lock().await;
        self.inner.state.move_session(id.into(),project).await?;
        self.broadcast_sessions().await; Ok(())
    }
    pub async fn delete_project(&self, id: String, revision: u64, mode: DeleteProjectMode) -> Result<()> {
        let _gate = self.inner.projects.lock().await;
        let sessions = self.inner.state.project_sessions(&id,revision).await?;
        let _deleting=(mode==DeleteProjectMode::DeleteChats).then(||self.deleting(sessions.clone()));
        let mut runtimes = Vec::new();
        if mode == DeleteProjectMode::DeleteChats {
            for session in &sessions {
                let Some(runtime)=self.inner.runtimes.lock().await.get(session).cloned() else {continue;};
                if let Some(agent) = &mut runtime.content.lock().await.agent { agent.stop(); }
                runtimes.push((session.clone(),runtime));
            }
        }
        let mut guards = Vec::new();
        for (session,runtime) in &runtimes {
            guards.push(runtime.operation.lock().await);
            self.retire_session(session,runtime).await;
        }
        // No partial project deletion: even a database failure preserves every
        // transcript. Work already cancelled above stays safely paused.
        self.inner.state.delete_project(id,revision,mode).await?;
        if mode == DeleteProjectMode::DeleteChats {
            for session in &sessions {
                self.inner.runtimes.lock().await.remove(session);

            }
        }
        self.maintain_uploads().await?;
        self.broadcast_projects().await?;
        self.broadcast_sessions().await;
        Ok(())
    }
}

#[cfg(test)]
#[path = "../tests/unit/projects.rs"]
mod tests;
