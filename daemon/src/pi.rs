use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{Mutex, broadcast, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;
use tracing::{debug, warn};

use crate::config::Config;

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const PROCESS_STOP_TIMEOUT: Duration = Duration::from_secs(12);
const EVENT_BUFFER: usize = 2048;

type ResponseSender = oneshot::Sender<std::result::Result<Value, String>>;

#[derive(Debug)]
pub struct UnconfirmedCommand(pub String);

impl std::fmt::Display for UnconfirmedCommand {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for UnconfirmedCommand {}

pub struct RpcProcess {
    shutdown_gate: Mutex<()>,
    writer: Mutex<Option<ChildStdin>>,
    child: Mutex<Option<Child>>,
    pending: Arc<StdMutex<HashMap<String, ResponseSender>>>,
    events: broadcast::Sender<Value>,
    dead: Arc<AtomicBool>,
    next_id: AtomicU64,
    stdout_task: Mutex<Option<JoinHandle<()>>>,
    stderr_task: Mutex<Option<JoinHandle<()>>>,
}

impl RpcProcess {
    pub fn spawn(config: &Config, session: Option<&str>) -> Result<Arc<Self>> {
        let mut command = Command::new(&config.pi_command);
        command
            .arg("--mode")
            .arg("rpc")
            .arg("--offline")
            .arg("--approve")
            .arg("--session-dir")
            .arg(&config.session_dir)
            .arg("--extension")
            .arg(&config.pi_extension_path)
            .env("TAU_ATTACHMENT_ROOT", &config.attachment_root)
            .env_remove("TAU_TOKEN")
            .current_dir(&config.cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(session) = session {
            command.arg("--session").arg(session);
        } else {
            command
                .arg("--model")
                .arg(&config.default_model)
                .arg("--thinking")
                .arg(&config.default_thinking_level);
        }

        let mut child = command.spawn().with_context(|| {
            format!("could not start Pi with {}", config.pi_command.display())
        })?;
        let stdin = child
            .stdin
            .take()
            .context("Pi stdin pipe was not created")?;
        let stdout = child
            .stdout
            .take()
            .context("Pi stdout pipe was not created")?;
        let stderr = child
            .stderr
            .take()
            .context("Pi stderr pipe was not created")?;

        let pending = Arc::new(StdMutex::new(HashMap::<String, ResponseSender>::new()));
        let (events, _) = broadcast::channel(EVENT_BUFFER);
        let dead = Arc::new(AtomicBool::new(false));

        let stdout_pending = pending.clone();
        let stdout_events = events.clone();
        let stdout_dead = dead.clone();
        let stdout_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut record = Vec::new();
            let end_reason = loop {
                record.clear();
                match reader.read_until(b'\n', &mut record).await {
                    Ok(0) => break "Pi stdout reached EOF".to_owned(),
                    Ok(_) => {
                        if record.last() == Some(&b'\n') {
                            record.pop();
                        }
                        if record.last() == Some(&b'\r') {
                            record.pop();
                        }
                        if record.iter().all(u8::is_ascii_whitespace) {
                            continue;
                        }
                        match serde_json::from_slice::<Value>(&record) {
                            Ok(message) => {
                                if message.get("type").and_then(Value::as_str) == Some("response")
                                    && let Some(id) = message.get("id").and_then(Value::as_str)
                                    && let Some(sender) = lock_unpoisoned(&stdout_pending).remove(id)
                                {
                                    let _ = sender.send(Ok(message));
                                    continue;
                                }
                                let _ = stdout_events.send(message);
                            }
                            Err(error) => warn!(%error, "ignoring malformed JSON from Pi stdout"),
                        }
                    }
                    Err(error) => break format!("failed reading Pi stdout: {error}"),
                }
            };
            if !stdout_dead.swap(true, Ordering::AcqRel) {
                fail_pending(&stdout_pending, &end_reason);
                let _ = stdout_events.send(serde_json::json!({ "type": "rpc_closed", "error": end_reason }));
            }
        });

        let stderr_task = tokio::spawn(async move {
            let mut reader = BufReader::new(stderr);
            let mut record = Vec::new();
            loop {
                record.clear();
                match reader.read_until(b'\n', &mut record).await {
                    Ok(0) => break,
                    Ok(_) => {
                        while matches!(record.last(), Some(b'\n' | b'\r')) {
                            record.pop();
                        }
                        if !record.is_empty() {
                            warn!(target: "pi_stderr", "{}", String::from_utf8_lossy(&record));
                        }
                    }
                    Err(error) => {
                        warn!(%error, "failed reading Pi stderr");
                        break;
                    }
                }
            }
        });

        Ok(Arc::new(Self {
            shutdown_gate: Mutex::new(()),
            writer: Mutex::new(Some(stdin)),
            child: Mutex::new(Some(child)),
            pending,
            events,
            dead,
            next_id: AtomicU64::new(1),
            stdout_task: Mutex::new(Some(stdout_task)),
            stderr_task: Mutex::new(Some(stderr_task)),
        }))
    }

    pub fn is_alive(&self) -> bool {
        !self.dead.load(Ordering::Acquire)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Value> {
        self.events.subscribe()
    }

    pub async fn request(&self, command: Value) -> Result<Value> {
        self.request_with_timeout(command, Some(COMMAND_TIMEOUT))
            .await
    }

    pub async fn request_unbounded(&self, command: Value) -> Result<Value> {
        self.request_with_timeout(command, None).await
    }

    pub async fn notify(&self, command: Value) -> Result<()> {
        self.write_command(&command).await
    }

    async fn request_with_timeout(
        &self,
        mut command: Value,
        response_timeout: Option<Duration>,
    ) -> Result<Value> {
        let id = format!("tau-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        command
            .as_object_mut()
            .context("Pi RPC command must be a JSON object")?
            .insert("id".to_owned(), Value::String(id.clone()));

        let (sender, receiver) = oneshot::channel();
        lock_unpoisoned(&self.pending).insert(id.clone(), sender);
        if let Err(error) = self.write_command(&command).await {
            lock_unpoisoned(&self.pending).remove(&id);
            return Err(UnconfirmedCommand(error.to_string()).into());
        }

        let response = if let Some(limit) = response_timeout {
            match timeout(limit, receiver).await {
                Ok(Ok(Ok(response))) => response,
                Ok(Ok(Err(error))) => return Err(UnconfirmedCommand(format!("Pi RPC channel closed: {error}")).into()),
                Ok(Err(_)) => return Err(UnconfirmedCommand("Pi response router stopped".to_owned()).into()),
                Err(_) => {
                    lock_unpoisoned(&self.pending).remove(&id);
                    return Err(UnconfirmedCommand("timed out waiting for Pi response".to_owned()).into());
                }
            }
        } else {
            match receiver.await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => return Err(UnconfirmedCommand(format!("Pi RPC channel closed: {error}")).into()),
                Err(_) => return Err(UnconfirmedCommand("Pi response router stopped".to_owned()).into()),
            }
        };

        if response.get("success").and_then(Value::as_bool) == Some(true) {
            Ok(response)
        } else {
            bail!(
                "{}",
                response
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("Pi rejected the RPC command")
            )
        }
    }

    async fn write_command(&self, command: &Value) -> Result<()> {
        command
            .as_object()
            .context("Pi RPC command must be a JSON object")?;
        if !self.is_alive() {
            bail!("Pi RPC channel is closed");
        }
        let mut encoded = serde_json::to_vec(command).context("could not encode Pi command")?;
        encoded.push(b'\n');
        let result = {
            let mut writer = self.writer.lock().await;
            match writer.as_mut() {
                Some(writer) => writer.write_all(&encoded).await,
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Pi stdin is closed",
                )),
            }
        };
        if let Err(error) = result {
            if !self.dead.swap(true, Ordering::AcqRel) {
                fail_pending(&self.pending, "Pi input pipe failed");
                let _ = self.events.send(serde_json::json!({ "type": "rpc_closed", "error": "Pi input pipe failed" }));
            }
            bail!("failed writing Pi command: {error}");
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        let _guard = self.shutdown_gate.lock().await;
        if !self.dead.swap(true, Ordering::AcqRel) {
            fail_pending(&self.pending, "Pi process was stopped");
            let _ = self.events.send(serde_json::json!({
                "type": "rpc_closed",
                "error": "Pi process was stopped"
            }));
        }

        self.writer.lock().await.take();
        if let Some(mut child) = self.child.lock().await.take()
            && timeout(PROCESS_STOP_TIMEOUT, child.wait()).await.is_err()
        {
            warn!("Pi did not stop gracefully; forcing termination");
            if let Err(error) = child.start_kill() {
                debug!(%error, "Pi process had already stopped");
            }
            let _ = child.wait().await;
        }

        for task_slot in [&self.stdout_task, &self.stderr_task] {
            let Some(mut task) = task_slot.lock().await.take() else {
                continue;
            };
            if timeout(PROCESS_STOP_TIMEOUT, &mut task).await.is_err() {
                task.abort();
                let _ = task.await;
            }
        }
    }
}

fn fail_pending(pending: &StdMutex<HashMap<String, ResponseSender>>, reason: &str) {
    for sender in lock_unpoisoned(pending).drain().map(|(_, sender)| sender) {
        let _ = sender.send(Err(reason.to_owned()));
    }
}

fn lock_unpoisoned<T>(mutex: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}
