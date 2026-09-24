//! Durable project metadata and atomic membership/deletion. The manager's project
//! gate orders membership changes against runtime retirement; SQLite commits first.
use anyhow::{Context, Result, ensure};
use rusqlite::{OptionalExtension, params};
use tau_protocol::{DeleteProjectMode, Project, GENERAL_PROJECT_ID, MAX_PROJECT_NAME_CHARS, MAX_PROJECT_PROMPT_CHARS};
use crate::{manager::AgentManager, state::StateStore};

fn validate(name: &str, prompt: &str) -> Result<()> {
    ensure!(!name.trim().is_empty() && name.chars().count() <= MAX_PROJECT_NAME_CHARS
        && !name.chars().any(char::is_control), "Project name must contain 1–{MAX_PROJECT_NAME_CHARS} characters on one line");
    ensure!(prompt.chars().count() <= MAX_PROJECT_PROMPT_CHARS, "Project prompt is too long");
    Ok(())
}
impl StateStore {
    pub async fn projects(&self) -> Result<Vec<Project>> {
        self.access(|db| {
            let mut query = db.prepare("SELECT id,name,prompt,revision FROM projects ORDER BY id='general' DESC,rowid")?;
            Ok(query.query_map([], |r| Ok(Project { id:r.get(0)?,name:r.get(1)?,prompt:r.get(2)?,revision:r.get(3)? }))?
                .collect::<rusqlite::Result<_>>()?)
        }).await
    }
    pub async fn create_project(&self, id: String, name: String, prompt: String) -> Result<()> {
        validate(&name, &prompt)?;
        uuid::Uuid::parse_str(&id).context("Invalid project ID")?;
        self.access(move |db| {
            let tx = db.transaction()?;
            ensure!(tx.query_row("SELECT count(*) FROM projects", [], |r| r.get::<_,u64>(0))? < 128, "At most 128 projects");
            tx.execute("INSERT INTO projects(id,name,prompt) VALUES(?1,?2,?3)", params![id,name.trim(),prompt])?;
            tx.commit()?; Ok(())
        }).await
    }
    pub async fn update_project(&self, id: String, revision: u64, name: String, prompt: String) -> Result<()> {
        validate(&name, &prompt)?;
        ensure!(id != GENERAL_PROJECT_ID || name == "General", "General cannot be renamed");
        ensure!(revision < i64::MAX as u64, "Project revision exhausted");
        self.access(move |db| {
            ensure!(db.execute("UPDATE projects SET name=?3,prompt=?4,revision=revision+1 WHERE id=?1 AND revision=?2",
                params![id,revision,name.trim(),prompt])? == 1, "Project changed or was deleted; reopen its settings");
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
            let prompt: String = tx.query_row("SELECT prompt FROM projects WHERE id=?1", [&project], |r| r.get(0)).context("Unknown project")?;
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
            ensure!(current == Some(revision), "Project changed or was deleted; reopen its settings");
            let mut query = db.prepare("SELECT id FROM sessions WHERE json_extract(data,'$.project_id')=?1 ORDER BY id")?;
            Ok(query.query_map([&id], |r| r.get(0))?.collect::<rusqlite::Result<_>>()?)
        }).await
    }
    pub async fn delete_project(&self, id: String, revision: u64, mode: DeleteProjectMode) -> Result<()> {
        ensure!(id != GENERAL_PROJECT_ID, "General cannot be deleted");
        self.access(move |db| {
            let tx = db.transaction()?;
            ensure!(tx.execute("DELETE FROM projects WHERE id=?1 AND revision=?2", params![id,revision])? == 1,
                "Project changed or was deleted; reopen its settings");
            match mode {
                DeleteProjectMode::MoveToGeneral => {
                    tx.execute("UPDATE sessions SET starter=0,data=json_set(data,'$.project_id','general','$.project_prompt',(SELECT prompt FROM projects WHERE id='general'),'$.starter',json('false')) WHERE json_extract(data,'$.project_id')=?1", [&id])?;
                }
                DeleteProjectMode::DeleteChats => {
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
    pub async fn projects_message(&self) -> Result<tau_protocol::ServerMessage> {
        Ok(tau_protocol::ServerMessage::Projects { projects:self.inner.state.projects().await? })
    }
    async fn broadcast_projects(&self) -> Result<()> {
        let _ = self.inner.events.send(self.projects_message().await?);
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
        let mut runtimes = Vec::new();
        if mode == DeleteProjectMode::DeleteChats {
            for session in &sessions {
                let runtime = self.runtime(session).await?;
                if let Some(agent) = &runtime.content.lock().await.agent { agent.cancel.cancel(); }
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
                if let Err(error) = tokio::fs::remove_dir_all(self.inner.config.upload_root.join(session)).await
                    && error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, "Deleted project chat uploads could not be removed");
                }
            }
        }
        self.broadcast_projects().await?;
        self.broadcast_sessions().await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn version_one_database_migrates_in_place_without_changing_history_or_prompt() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("old.sqlite3");
        let db = rusqlite::Connection::open(&path).unwrap();
        db.execute_batch(include_str!("schema.sql")).unwrap();
        let data = serde_json::json!({"title":"Existing","starter":true,"parent_id":null,"model":{"provider":"openai-codex","modelId":"gpt-6-astra"},
            "thinking":"high","created_at_ms":12,"updated_at_ms":20,"tokens":null,"needs_turn":false,"head":"entry","next_order":0,"revision":3}).to_string();
        db.execute("INSERT INTO sessions(id,starter,activity,data,queue) VALUES('old',1,20,?1,?2)",params![data,serde_json::to_string(&tau_protocol::QueueState::native()).unwrap()]).unwrap();
        db.execute("INSERT INTO entries(session_id,id,kind,data) VALUES('old','entry','model_change',?1)",
            [r#"{"id":"entry","type":"model_change","provider":"openai-codex","modelId":"gpt-6-astra","thinkingLevel":"high"}"#]).unwrap();
        drop(db);
        let state = StateStore::load(path.clone()).await.unwrap();
        let old = state.get("old").await.unwrap().unwrap();
        assert_eq!((old.project_id.as_str(),old.project_prompt.as_str()),("general",""));
        assert_eq!((old.revision,old.updated_at_ms,old.head.as_deref()),(3,20,Some("entry")));
        assert_eq!(state.projects().await.unwrap(),vec![Project::general()]);
        let project = uuid::Uuid::new_v4().to_string();
        state.create_project(project.clone(),"Build".into(),"Pinned".into()).await.unwrap();
        let chat = state.create(old.model.clone(),old.thinking.clone(),None,project.clone()).await.unwrap();
        assert_ne!(chat,"old");
        assert_eq!(state.create(old.model,old.thinking,None,"general".into()).await.unwrap(),"old");
        state.update_project(project.clone(),0,"Build".into(),"Edited".into()).await.unwrap();
        assert_eq!(state.project_prompt(&chat).await.unwrap(),"Pinned");
        state.move_session(chat.clone(),project).await.unwrap();
        assert_eq!(state.project_prompt(&chat).await.unwrap(),"Pinned","Moving to the current project is not an apply-latest backdoor");
        let again = StateStore::load(path).await.unwrap();
        assert_eq!(again.project_prompt(&chat).await.unwrap(),"Pinned");
        again.access(|db| {
            assert_eq!(db.query_row("PRAGMA user_version",[],|r|r.get::<_,u32>(0))?,2);
            assert_eq!(db.query_row("SELECT count(*) FROM entries WHERE session_id='old'",[],|r|r.get::<_,u32>(0))?,1);
            Ok(())
        }).await.unwrap();
    }
}
