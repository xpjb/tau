use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

#[derive(Clone)]
pub struct StateStore {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    state: RwLock<PersistedState>,
    write_gate: Mutex<()>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct PersistedState {
    schema: u32,
    #[serde(default)]
    sessions: BTreeMap<String, StoredSession>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            schema: 1,
            sessions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredSession {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

impl StateStore {
    pub async fn load(path: PathBuf) -> Result<Self> {
        let state = match fs::read(&path).await {
            Ok(bytes) => serde_json::from_slice::<PersistedState>(&bytes)
                .with_context(|| format!("failed to parse {}", path.display()))?,
            Err(error) if error.kind() == ErrorKind::NotFound => PersistedState::default(),
            Err(error) => {
                return Err(error).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        if state.schema != 1 {
            bail!("unsupported Tau state schema {}", state.schema);
        }

        Ok(Self {
            inner: Arc::new(Inner {
                path,
                state: RwLock::new(state),
                write_gate: Mutex::new(()),
            }),
        })
    }

    pub fn list(&self) -> Vec<(String, StoredSession)> {
        let mut sessions = self
            .read_state()
            .sessions
            .iter()
            .map(|(id, session)| (id.clone(), session.clone()))
            .collect::<Vec<_>>();
        sessions.sort_by(|left, right| {
            right
                .1
                .updated_at_ms
                .cmp(&left.1.updated_at_ms)
                .then_with(|| left.0.cmp(&right.0))
        });
        sessions
    }

    pub fn get(&self, id: &str) -> Option<StoredSession> {
        self.read_state().sessions.get(id).cloned()
    }

    pub async fn create(
        &self,
        title: String,
        parent_id: Option<String>,
        session_file: Option<String>,
    ) -> Result<String> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let id = Uuid::new_v4().to_string();
        let now = now_ms();
        state.sessions.insert(
            id.clone(),
            StoredSession {
                title,
                session_file,
                parent_id,
                created_at_ms: now,
                updated_at_ms: now,
            },
        );
        self.commit(state).await?;
        Ok(id)
    }

    pub async fn set_session_file(&self, id: &str, session_file: String) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let session = state
            .sessions
            .get_mut(id)
            .with_context(|| format!("unknown session {id}"))?;
        session.session_file = Some(session_file);
        self.commit(state).await
    }

    pub async fn rename(&self, id: &str, title: String) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let session = state
            .sessions
            .get_mut(id)
            .with_context(|| format!("unknown session {id}"))?;
        session.title = title;
        session.updated_at_ms = now_ms();
        self.commit(state).await
    }

    pub async fn touch(&self, id: &str) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let session = state
            .sessions
            .get_mut(id)
            .with_context(|| format!("unknown session {id}"))?;
        session.updated_at_ms = now_ms();
        self.commit(state).await
    }

    pub async fn remove(&self, id: &str) -> Result<StoredSession> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let removed = state
            .sessions
            .remove(id)
            .with_context(|| format!("unknown session {id}"))?;
        for session in state.sessions.values_mut() {
            if session.parent_id.as_deref() == Some(id) {
                session.parent_id = None;
            }
        }
        self.commit(state).await?;
        Ok(removed)
    }

    fn read_state(&self) -> std::sync::RwLockReadGuard<'_, PersistedState> {
        self.inner
            .state
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    async fn commit(&self, state: PersistedState) -> Result<()> {
        let path = &self.inner.path;
        if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        let file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("state.json");
        let temporary = path.with_file_name(format!(".{file_name}.tmp"));
        let mut bytes = serde_json::to_vec_pretty(&state).context("failed to encode Tau state")?;
        bytes.push(b'\n');

        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&temporary)
            .await
            .with_context(|| format!("failed to open {}", temporary.display()))?;
        file.write_all(&bytes)
            .await
            .with_context(|| format!("failed to write {}", temporary.display()))?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        fs::rename(&temporary, path).await.with_context(|| {
            format!(
                "failed to replace {} with {}",
                path.display(),
                temporary.display()
            )
        })?;
        *self
            .inner
            .state
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = state;
        Ok(())
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn persists_session_identity_and_lineage() {
        let root = std::env::temp_dir().join(format!("tau-state-{}", Uuid::new_v4()));
        let path = root.join("state.json");
        let store = StateStore::load(path.clone()).await.unwrap();
        let parent = store
            .create("First".to_owned(), None, Some("/tmp/first.jsonl".to_owned()))
            .await
            .unwrap();
        let child = store
            .create(
                "Fork of First".to_owned(),
                Some(parent.clone()),
                Some("/tmp/child.jsonl".to_owned()),
            )
            .await
            .unwrap();
        store.rename(&child, "Alternative".to_owned()).await.unwrap();

        let reloaded = StateStore::load(path).await.unwrap();
        let child_state = reloaded.get(&child).unwrap();
        assert_eq!(child_state.title, "Alternative");
        assert_eq!(child_state.parent_id.as_deref(), Some(parent.as_str()));
        assert_eq!(
            child_state.session_file.as_deref(),
            Some("/tmp/child.jsonl")
        );

        let removed = reloaded.remove(&parent).await.unwrap();
        assert_eq!(removed.title, "First");
        let reloaded = StateStore::load(reloaded.inner.path.clone()).await.unwrap();
        assert!(reloaded.get(&parent).is_none());
        assert!(reloaded.get(&child).unwrap().parent_id.is_none());
        fs::remove_dir_all(root).await.unwrap();
    }
}
