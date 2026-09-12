use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tokio::fs::{self, OpenOptions};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use uuid::Uuid;

pub(crate) const DEFAULT_TITLE_PROMPT: &str = include_str!("../../scripts/title_prompt.txt");
pub(crate) const MAX_FLAG_CHARS: usize = 4096;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Flag {
    pub id: String,
    pub timestamp_ms: u64,
    pub session_id: String,
    pub session_title: String,
    pub text: String,
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title_prompt: Option<String>,
    #[serde(default)]
    sessions: BTreeMap<String, StoredSession>,
}

impl Default for PersistedState {
    fn default() -> Self {
        Self {
            schema: 1,
            title_prompt: None,
            sessions: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModel {
    pub provider: String,
    pub model_id: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StoredSession {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<SessionModel>,
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

        let store = Self {
            inner: Arc::new(Inner {
                path,
                state: RwLock::new(state),
                write_gate: Mutex::new(()),
            }),
        };
        store.backfill_models().await?;
        Ok(store)
    }

    pub async fn title_prompt(&self, replacement: Option<String>) -> Result<String> {
        if let Some(prompt) = replacement {
            let _guard = self.inner.write_gate.lock().await;
            let mut state = self.read_state().clone();
            if state.title_prompt.as_ref() != Some(&prompt) {
                state.title_prompt = Some(prompt);
                self.commit(state).await?;
            }
        }
        Ok(self.read_state().title_prompt.as_deref().unwrap_or(DEFAULT_TITLE_PROMPT).to_owned())
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
        model: Option<SessionModel>,
    ) -> Result<String> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let id = Uuid::new_v4().to_string();
        let now = next_activity_ms(&state);
        state.sessions.insert(
            id.clone(),
            StoredSession {
                title,
                session_file,
                parent_id,
                model,
                created_at_ms: now,
                updated_at_ms: now,
            },
        );
        self.commit(state).await?;
        Ok(id)
    }

    pub async fn flag(&self, id: &str, text: &str) -> Result<Flag> {
        if text.trim().is_empty() || text.chars().count() > MAX_FLAG_CHARS {
            bail!("Flag text must contain 1–{MAX_FLAG_CHARS} characters");
        }
        let _guard = self.inner.write_gate.lock().await;
        let session = self.get(id).with_context(|| format!("unknown session {id}"))?;
        let flag = Flag {
            id: Uuid::new_v4().to_string(),
            timestamp_ms: SystemTime::now().duration_since(UNIX_EPOCH)?.as_millis().try_into()?,
            session_id: id.to_owned(),
            session_title: session.title,
            text: text.to_owned(),
        };
        let mut encoded = serde_json::to_vec(&flag)?;
        encoded.push(b'\n');
        let path = self.inner.path.with_file_name("flags.jsonl");
        if path == self.inner.path { bail!("Tau state and flag log paths must differ"); }
        let mut options = OpenOptions::new();
        options.create(true).append(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options.open(&path).await.context("could not open Tau flag log")?;
        let length = file.metadata().await?.len();
        let saved = async {
            file.write_all(&encoded).await?;
            file.sync_data().await
        }.await;
        if let Err(error) = saved {
            file.set_len(length).await.context("could not remove an incomplete flag entry")?;
            file.sync_data().await.context("could not sync the flag log after a failed append")?;
            return Err(error).context("could not save Tau flag");
        }
        Ok(flag)
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

    pub async fn set_model(&self, id: &str, model: SessionModel) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let session = state
            .sessions
            .get_mut(id)
            .with_context(|| format!("unknown session {id}"))?;
        if session.model.as_ref() == Some(&model) {
            return Ok(());
        }
        session.model = Some(model);
        self.commit(state).await
    }

    pub async fn rename(&self, id: &str, title: String) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let updated = next_activity_ms(&state);
        let session = state
            .sessions
            .get_mut(id)
            .with_context(|| format!("unknown session {id}"))?;
        session.title = title;
        session.updated_at_ms = updated;
        self.commit(state).await
    }

    pub async fn touch(&self, id: &str) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let updated = next_activity_ms(&state);
        let session = state
            .sessions
            .get_mut(id)
            .with_context(|| format!("unknown session {id}"))?;
        session.updated_at_ms = updated;
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

    async fn backfill_models(&self) -> Result<()> {
        let _guard = self.inner.write_gate.lock().await;
        let mut state = self.read_state().clone();
        let mut changed = false;
        for session in state.sessions.values_mut() {
            if session.model.is_some() {
                continue;
            }
            let Some(path) = session.session_file.as_deref() else {
                continue;
            };
            if let Ok(Some(model)) = crate::transcript::session_model_from_file(Path::new(path)).await {
                session.model = Some(model);
                changed = true;
            }
        }
        if changed {
            self.commit(state).await?;
        }
        Ok(())
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

fn next_activity_ms(state: &PersistedState) -> u64 {
    let now: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX);
    now.max(state.sessions.values().map(|session| session.updated_at_ms).max().unwrap_or(0).saturating_add(1))
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
            .create(
                "First".to_owned(),
                None,
                Some("/tmp/first.jsonl".to_owned()),
                Some(SessionModel {
                    provider: "openai-codex".to_owned(),
                    model_id: "gpt-5.6-sol".to_owned(),
                }),
            )
            .await
            .unwrap();
        let child = store
            .create(
                "Fork of First".to_owned(),
                Some(parent.clone()),
                Some("/tmp/child.jsonl".to_owned()),
                None,
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
        assert_eq!(
            reloaded.get(&parent).unwrap().model,
            Some(SessionModel {
                provider: "openai-codex".to_owned(),
                model_id: "gpt-5.6-sol".to_owned(),
            }),
        );

        let removed = reloaded.remove(&parent).await.unwrap();
        assert_eq!(removed.title, "First");
        let reloaded = StateStore::load(reloaded.inner.path.clone()).await.unwrap();
        assert!(reloaded.get(&parent).is_none());
        assert!(reloaded.get(&child).unwrap().parent_id.is_none());
        fs::remove_dir_all(root).await.unwrap();
    }

    #[tokio::test]
    async fn backfills_the_model_on_the_active_branch() {
        let root = std::env::temp_dir().join(format!("tau-model-state-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).await.unwrap();
        let session_path = root.join("session.jsonl");
        fs::write(
            &session_path,
            concat!(
                "{\"type\":\"session\",\"version\":3}\n",
                "{\"type\":\"model_change\",\"id\":\"root-model\",\"parentId\":null,\"provider\":\"openai-codex\",\"modelId\":\"gpt-5.6-sol\"}\n",
                "{\"type\":\"message\",\"id\":\"first\",\"parentId\":\"root-model\",\"message\":{\"role\":\"user\",\"content\":\"hello\"}}\n",
                "{\"type\":\"model_change\",\"id\":\"abandoned-model\",\"parentId\":\"first\",\"provider\":\"openrouter\",\"modelId\":\"z-ai/glm-5.3-flash\"}\n",
                "{\"type\":\"message\",\"id\":\"abandoned\",\"parentId\":\"abandoned-model\",\"message\":{\"role\":\"user\",\"content\":\"branch\"}}\n",
                "{\"type\":\"message\",\"id\":\"active\",\"parentId\":\"first\",\"message\":{\"role\":\"user\",\"content\":\"active\"}}\n",
            ),
        )
        .await
        .unwrap();
        let state_path = root.join("state.json");
        fs::write(
            &state_path,
            format!(
                "{{\"schema\":1,\"sessions\":{{\"chat\":{{\"title\":\"Chat\",\"session_file\":{},\"created_at_ms\":1,\"updated_at_ms\":1}}}}}}",
                serde_json::to_string(&session_path.to_string_lossy()).unwrap(),
            ),
        )
        .await
        .unwrap();

        let store = StateStore::load(state_path.clone()).await.unwrap();
        assert_eq!(
            store.get("chat").unwrap().model,
            Some(SessionModel {
                provider: "openai-codex".to_owned(),
                model_id: "gpt-5.6-sol".to_owned(),
            }),
        );
        let persisted: serde_json::Value =
            serde_json::from_slice(&fs::read(state_path).await.unwrap()).unwrap();
        assert_eq!(
            persisted["sessions"]["chat"]["model"]["modelId"],
            "gpt-5.6-sol",
        );
        fs::remove_dir_all(root).await.unwrap();
    }
}
