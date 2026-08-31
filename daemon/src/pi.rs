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
                                if route_response(&stdout_pending, &message) {
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
            mark_dead(&stdout_pending, &stdout_events, &stdout_dead, end_reason);
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

    async fn request_with_timeout(
        &self,
        mut command: Value,
        response_timeout: Option<Duration>,
    ) -> Result<Value> {
        if !self.is_alive() {
            bail!("Pi RPC channel is closed");
        }

        let id = format!("tau-{}", self.next_id.fetch_add(1, Ordering::Relaxed));
        command
            .as_object_mut()
            .context("Pi RPC command must be a JSON object")?
            .insert("id".to_owned(), Value::String(id.clone()));
        let mut encoded = serde_json::to_vec(&command).context("could not encode Pi command")?;
        encoded.push(b'\n');

        let (sender, receiver) = oneshot::channel();
        lock_unpoisoned(&self.pending).insert(id.clone(), sender);
        let write_result = {
            let mut writer = self.writer.lock().await;
            match writer.as_mut() {
                Some(writer) => writer.write_all(&encoded).await,
                None => Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "Pi stdin is closed",
                )),
            }
        };
        if let Err(error) = write_result {
            lock_unpoisoned(&self.pending).remove(&id);
            self.dead.store(true, Ordering::Release);
            bail!("failed writing Pi command: {error}");
        }

        let response = if let Some(limit) = response_timeout {
            match timeout(limit, receiver).await {
                Ok(Ok(Ok(response))) => response,
                Ok(Ok(Err(error))) => bail!("Pi RPC channel closed: {error}"),
                Ok(Err(_)) => bail!("Pi response router stopped"),
                Err(_) => {
                    lock_unpoisoned(&self.pending).remove(&id);
                    bail!("timed out waiting for Pi response");
                }
            }
        } else {
            match receiver.await {
                Ok(Ok(response)) => response,
                Ok(Err(error)) => bail!("Pi RPC channel closed: {error}"),
                Err(_) => bail!("Pi response router stopped"),
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

fn route_response(pending: &StdMutex<HashMap<String, ResponseSender>>, message: &Value) -> bool {
    if message.get("type").and_then(Value::as_str) != Some("response") {
        return false;
    }
    let Some(id) = message.get("id").and_then(Value::as_str) else {
        return false;
    };
    let Some(sender) = lock_unpoisoned(pending).remove(id) else {
        return false;
    };
    let _ = sender.send(Ok(message.clone()));
    true
}

fn mark_dead(
    pending: &StdMutex<HashMap<String, ResponseSender>>,
    events: &broadcast::Sender<Value>,
    dead: &AtomicBool,
    reason: String,
) {
    if dead.swap(true, Ordering::AcqRel) {
        return;
    }
    fail_pending(pending, &reason);
    let _ = events.send(serde_json::json!({ "type": "rpc_closed", "error": reason }));
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
