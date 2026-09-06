#![cfg(unix)]

use std::collections::VecDeque;
use std::os::unix::fs::{PermissionsExt, symlink};
use std::path::PathBuf;

use super::*;
use crate::protocol::QueueOperation;
use crate::transcript::{EntryPhase, EntryRole, QueueRef, TranscriptSnapshot};

async fn fixture() -> (AgentManager, PathBuf) {
    let root = std::env::temp_dir().join(format!("tau-manager-{}", uuid::Uuid::new_v4()));
    fs::create_dir_all(root.join("pi-sessions")).await.unwrap();
    fs::create_dir_all(root.join("outbox")).await.unwrap();
    let mock = root.join("pi.py");
    fs::write(&mock, include_str!("../tests/fixtures/pi.py")).await.unwrap();
    fs::set_permissions(&mock, std::fs::Permissions::from_mode(0o700)).await.unwrap();
    let config = Config {
        bind: "127.0.0.1:0".parse().unwrap(),
        token: Arc::from("test-token-with-at-least-thirty-two-characters"),
        pi_command: mock, default_model: "test/model".to_owned(), default_thinking_level: "high".to_owned(),
        cwd: root.clone(), state_path: root.join("state.json"), session_dir: root.join("pi-sessions"),
        telemetry_path: root.join("crashes.jsonl"), pi_extension_path: root.join("extension.ts"),
        attachment_root: root.join("outbox"), upload_root: root.join("uploads"),
    };
    let state = StateStore::load(config.state_path.clone()).await.unwrap();
    (AgentManager::new(config, state), root)
}

struct Events {
    metadata: broadcast::Receiver<ServerMessage>,
    initial: VecDeque<ServerMessage>,
    transcript: Option<broadcast::Receiver<Arc<ServerMessage>>>,
}

impl Events {
    fn new(manager: &AgentManager) -> Self {
        Self { metadata: manager.subscribe(), initial: VecDeque::new(), transcript: None }
    }

    async fn open(&mut self, manager: &AgentManager, id: &str) {
        let feed = manager.open_session(id, &[], &[]).await.unwrap();
        self.initial = feed.initial.into();
        self.transcript = Some(feed.events);
    }
}

async fn receive(events: &mut Events, predicate: impl Fn(&ServerMessage) -> bool) -> ServerMessage {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = if let Some(message) = events.initial.pop_front() { Ok(message) } else {
                tokio::select! {
                    biased;
                    message = events.transcript.as_mut().unwrap().recv() => message.map(Arc::unwrap_or_clone),
                    message = events.metadata.recv() => message,
                }
            };
            match message {
                Ok(message) if predicate(&message) => break message,
                Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(error) => panic!("event channel closed: {error}"),
            }
        }
    }).await.expect("expected daemon event did not arrive")
}

#[tokio::test]
async fn drives_transcript_controls_recovery_and_process_replacement_through_rpc() {
    let (manager, root) = fixture().await;
    let mut events = Events::new(&manager);
    let id = manager.create_session().await.unwrap();
    events.open(&manager, &id).await;
    let runtime = manager.runtime(&id).await.unwrap();
    assert!(runtime.content.lock().await.process.is_none());
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptSnapshot { snapshot, .. } if snapshot.entries.is_empty())).await;
    assert!(matches!(manager.prompt(&id, "Say hello", "request-1").await.unwrap().disposition, PromptDisposition::Submitted));
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Entry { entry }, .. } if entry.role == Some(EntryRole::Assistant) && entry.phase == EntryPhase::Saved)).await;
    receive(&mut events, |event| matches!(event, ServerMessage::SessionState { status: SessionStatus::Idle, .. })).await;
    let snapshot = runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]);
    assert_eq!(snapshot.entries.len(), 2);
    assert_eq!(runtime.snapshot().context_usage, Some(ContextUsage { tokens: Some(64000), context_window: 200000 }));
    assert_eq!(snapshot.entries[0].origin.request_id.as_deref(), Some("request-1"));
    assert_eq!(snapshot.entries[1].content[0].text, "Checking");
    assert_eq!(snapshot.entries[1].content[1].text, "Hello from Tau");
    assert!(snapshot.entries[1].origin.stream_id.is_some());
    let commands = manager.commands(&id).await.unwrap();
    assert!(commands.iter().any(|command| command.name == "choose" && command.source == SlashCommandSource::Extension));
    assert!(commands.iter().any(|command| command.name == "review" && command.source == SlashCommandSource::Prompt));
    assert!(commands.iter().any(|command| command.name == "model" && command.arguments.iter().any(|argument| argument.value == "test/model")));
    assert!(commands.iter().all(|command| command.name != INTERNAL_FORK_COMMAND));
    let args = fs::read_to_string(root.join("pi-sessions/spawn-args")).await.unwrap();
    let args = serde_json::from_str::<Vec<String>>(args.lines().next().unwrap()).unwrap();
    assert!(args.windows(2).any(|pair| pair == ["--model", "test/model"]));
    assert!(args.windows(2).any(|pair| pair == ["--thinking", "high"]));
    let dialog_manager = manager.clone();
    let dialog_id = id.clone();
    let prompt = tokio::spawn(async move { dialog_manager.prompt(&dialog_id, "/choose", "dialog-prompt").await });
    receive(&mut events, |event| matches!(event, ServerMessage::ExtensionUi { .. })).await;
    assert!(manager.extension_ui_response(&id, "dialog-1", Some("invalid".to_owned()), None, false).await.is_err());
    manager.extension_ui_response(&id, "dialog-1", Some("Two".to_owned()), None, false).await.unwrap();
    assert!(matches!(prompt.await.unwrap().unwrap().disposition, PromptDisposition::Handled));
    assert_eq!(fs::read_to_string(root.join("pi-sessions/extension-response")).await.unwrap(), "Two");
    assert!(matches!(manager.prompt(&id, "/thinking high", "thinking").await.unwrap().disposition, PromptDisposition::Handled));
    manager.prompt(&id, "/model test/other", "model").await.unwrap();
    assert_eq!(manager.inner.state.get(&id).unwrap().model.unwrap().model_id, "other");
    assert_eq!(runtime.snapshot().context_usage.unwrap().context_window, 128000);
    manager.prompt(&id, "/compact", "compact").await.unwrap();
    assert_eq!(runtime.snapshot().context_usage, Some(ContextUsage { tokens: None, context_window: 128000 }));
    let process = runtime.content.lock().await.process.clone().unwrap();
    process.request(json!({"type":"mock_context", "usage":{"tokens":32000,"contextWindow":128000}})).await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::SessionState { context_usage: Some(usage), .. } if usage.tokens == Some(32000))).await;
    events.open(&manager, &id).await;
    receive(&mut events, |event| matches!(event, ServerMessage::SessionState { context_usage: Some(usage), .. } if usage.tokens == Some(32000))).await;
    let ServerMessage::Sessions { sessions } = manager.sessions_message().await else { unreachable!() };
    assert_eq!(sessions.iter().find(|session| session.id == id).unwrap().context_usage, runtime.snapshot().context_usage);
    for usage in [Value::Null, json!({"tokens": -1, "contextWindow": 128000}), json!({"tokens":1,"contextWindow":0})] {
        process.request(json!({"type":"mock_context", "usage":usage})).await.unwrap();
        manager.refresh_runtime_status(&id, &runtime, &process).await.unwrap();
        assert!(runtime.snapshot().context_usage.is_none());
    }
    process.request(json!({"type":"mock_context", "usage":{"tokens":32000,"contextWindow":128000}})).await.unwrap();
    assert!(manager.prompt(&id, "/settings", "settings").await.is_err());

    manager.prompt(&id, "hold", "request-2").await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Entry { entry }, .. } if entry.phase == EntryPhase::Live)).await;
    let process = runtime.content.lock().await.process.clone().unwrap();
    manager.prompt(&id, "same", "q1").await.unwrap();
    manager.prompt(&id, "same", "q2").await.unwrap();
    assert_eq!(runtime.snapshot().status, SessionStatus::Running);
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Queue { queue }, .. } if queue.requests.len() == 2)).await;
    let snapshot = runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]);
    assert_eq!(snapshot.queue.requests[0].request_id, "q1");
    assert_eq!(snapshot.queue.requests[1].request_id, "q2");
    assert_eq!(manager.queue_control(&id, &snapshot.generation, "edit", QueueOperation::Edit { request_id: "q1".to_owned(), revision: 0, text: "edited".to_owned() }).await.unwrap(), "edited");
    assert_eq!(manager.queue_control(&id, &snapshot.generation, "stale", QueueOperation::Delete { request_id: "q1".to_owned(), revision: 0 }).await.unwrap(), "conflict");
    assert!(manager.queue_control(&id, "retired", "stale-generation", QueueOperation::Resume { run_id: snapshot.queue.run_id.clone() }).await.is_err());
    manager.queue_control(&id, &snapshot.generation, "prefix", QueueOperation::Prefix {
        run_id: snapshot.queue.run_id.clone(), boundary: "reasoning_checkpoint".to_owned(),
        requests: vec![QueueRef { request_id: "q1".to_owned(), revision: 1 }],
    }).await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Queue { queue }, .. } if queue.control.as_ref().is_some_and(|control| control.command_id == "prefix"))).await;
    events.open(&manager, &id).await;
    let reconnect = receive(&mut events, |event| matches!(event, ServerMessage::TranscriptSnapshot { snapshot, .. } if snapshot.queue.control.as_ref().is_some_and(|control| control.command_id == "prefix"))).await;
    if let ServerMessage::TranscriptSnapshot { snapshot, .. } = reconnect {
        assert_eq!(snapshot.queue.control.unwrap().status, "waiting");
        assert_eq!(snapshot.queue.requests[0].text, "edited");
    }
    assert_eq!(manager.queue_control(&id, &snapshot.generation, "cancel", QueueOperation::Cancel { control_id: "prefix".to_owned() }).await.unwrap(), "cancelled");
    assert_eq!(manager.queue_control(&id, &snapshot.generation, "delete", QueueOperation::Delete { request_id: "q2".to_owned(), revision: 0 }).await.unwrap(), "deleted");
    let old_snapshot = process.request(json!({"type":"get_transcript"})).await.unwrap();
    process.request(json!({"type":"mock_gap"})).await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::ResyncRequired { .. })).await;
    assert_eq!(runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]).entries.last().unwrap().content[0].text, "Checking.");
    {
        let mut content = runtime.content.lock().await;
        let generation = content.transcript.as_ref().unwrap().generation.clone();
        assert!(manager.replace_transcript(&id, &mut content, &old_snapshot["data"]).await.is_err());
        assert_eq!(content.transcript.as_ref().unwrap().generation, generation);
    }
    let before_lag = runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]);
    {
        let _content = runtime.content.lock().await;
        process.request(json!({"type":"mock_lag"})).await.unwrap();
    }
    receive(&mut events, |event| matches!(event, ServerMessage::ResyncRequired { .. })).await;
    let after_lag = runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]);
    assert_ne!(before_lag.generation, after_lag.generation);
    assert_eq!(after_lag.queue.requests.len(), 1);
    let error = process.request(json!({"type":"mock_reject"})).await.unwrap_err();
    assert!(!error.is::<crate::pi::UnconfirmedCommand>());
    manager.abort(&id).await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::SessionState { status: SessionStatus::Idle, .. })).await;
    manager.sleep_if_idle(&id, runtime.snapshot().idle_since.unwrap()).await;
    assert!(runtime.content.lock().await.process.is_some());
    let error = process.request(json!({"type":"mock_exit"})).await.unwrap_err();
    assert!(error.is::<crate::pi::UnconfirmedCommand>());
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Interrupted, .. })).await;
    events.open(&manager, &id).await;
    let interrupted = runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]);
    assert_eq!(interrupted.entries.last().unwrap().phase, EntryPhase::Interrupted);
    assert_eq!(interrupted.entries.last().unwrap().content[0].text.len(), 2109);
    assert!(!interrupted.queue.available);
    manager.commands(&id).await.unwrap();
    let restarted = runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]);
    assert_ne!(interrupted.generation, restarted.generation);
    assert_eq!(restarted.entries.last().unwrap().content[0].text.len(), 2109);
    assert_eq!(restarted.entries.last().unwrap().phase, EntryPhase::Interrupted);
    assert!(restarted.queue.requests.is_empty());
    let (late, retired_events) = broadcast::channel(4);
    late.send(json!({"type":"agent_start"})).unwrap();
    tokio::time::timeout(Duration::from_secs(1), manager.monitor_process(id.clone(), process, retired_events)).await.unwrap();
    assert_eq!(runtime.snapshot().status, SessionStatus::Idle);
    assert_eq!(runtime.content.lock().await.transcript.as_ref().unwrap().generation, restarted.generation);

    manager.queue_control(&id, &restarted.generation, "pause", QueueOperation::Pause { run_id: None, boundary: "turn".to_owned() }).await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Queue { queue }, .. } if queue.paused)).await;
    assert!(matches!(manager.prompt(&id, "held while idle", "idle-held").await.unwrap().disposition, PromptDisposition::Queued));
    assert_eq!(runtime.snapshot().status, SessionStatus::Idle);
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Queue { queue }, .. } if queue.requests.iter().any(|request| request.request_id == "idle-held"))).await;
    assert_eq!(manager.queue_control(&id, &restarted.generation, "delete-idle", QueueOperation::Delete { request_id: "idle-held".to_owned(), revision: 0 }).await.unwrap(), "deleted");
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Queue { queue }, .. } if queue.paused && queue.requests.is_empty())).await;
    manager.sleep_if_idle(&id, runtime.snapshot().idle_since.unwrap()).await;
    assert!(runtime.content.lock().await.process.is_some());
    assert!(manager.clone_session(&id).await.is_err());
    manager.queue_control(&id, &restarted.generation, "resume", QueueOperation::Resume { run_id: None }).await.unwrap();
    receive(&mut events, |event| matches!(event, ServerMessage::TranscriptUpdate { change: TranscriptChange::Queue { queue }, .. } if !queue.paused && queue.control.as_ref().is_some_and(|control| control.action == "resume"))).await;
    let child = manager.clone_session(&id).await.unwrap();
    events.open(&manager, &child).await;
    let child_runtime = manager.runtime(&child).await.unwrap();
    assert_eq!(child_runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]).entries.len(), 3);
    events.open(&manager, &id).await;
    assert_eq!(runtime.content.lock().await.transcript.as_ref().unwrap().snapshot(&[], &[]).entries.last().unwrap().phase, EntryPhase::Interrupted);
    manager.shutdown().await;
    fs::remove_dir_all(root).await.unwrap();
}

#[tokio::test]
async fn preserves_cold_jsonl_attachments_and_path_boundaries() {
    let (manager, root) = fixture().await;
    let id = manager.create_session().await.unwrap();
    let path = root.join("pi-sessions/cold.jsonl");
    let artifact = root.join("outbox/result.zip");
    fs::write(&artifact, b"artifact").await.unwrap();
    let outside = root.join("private.bin");
    fs::write(&outside, b"private").await.unwrap();
    symlink(&outside, root.join("outbox/link.bin")).unwrap();
    fs::write(root.join("outbox/bad.png"), b"not an image").await.unwrap();
    let entries = [
        json!({"type":"message","id":"artifact","parentId":null,"message":{"role":"toolResult","content":[{"type":"text","text":"queued"}],"details":{"tauAttachment":{"version":1,"kind":"file","path":artifact,"caption":"Build"}}}}),
        json!({"type":"message","id":"link","parentId":"artifact","message":{"role":"toolResult","details":{"tauAttachment":{"version":1,"kind":"file","path":root.join("outbox/link.bin")}}}}),
        json!({"type":"message","id":"image","parentId":"artifact","message":{"role":"toolResult","details":{"tauAttachment":{"version":1,"kind":"image","path":root.join("outbox/bad.png")}}}}),
    ];
    let original = entries.iter().map(|entry| format!("{entry}\n")).collect::<String>();
    fs::write(&path, &original).await.unwrap();
    manager.inner.state.set_session_file(&id, path.to_string_lossy().into_owned()).await.unwrap();
    let mut events = Events::new(&manager);
    events.open(&manager, &id).await;
    let ServerMessage::TranscriptSnapshot { snapshot: TranscriptSnapshot { entries, head, .. }, .. } =
        receive(&mut events, |event| matches!(event, ServerMessage::TranscriptSnapshot { .. })).await else { unreachable!() };
    assert_eq!(head.as_deref(), Some("image"));
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].attachment.as_ref().unwrap().size, Some(8));
    assert_eq!(entries[0].attachment.as_ref().unwrap().file_name, "result.zip");
    assert_eq!(manager.resolve_attachment(&id, "artifact").await.unwrap().size, 8);
    assert!(manager.resolve_attachment(&id, "link").await.is_err());
    assert!(manager.resolve_attachment(&id, "image").await.is_err());
    assert!(manager.resolve_attachment(&id, "missing").await.is_err());
    let uploaded = manager.store_upload(&id, "../source file.rs", b"fn main() {}\n").await.unwrap();
    assert_eq!(uploaded.name, "source_file.rs");
    assert!(PathBuf::from(&uploaded.path).starts_with(root.join("uploads").join(&id)));
    assert!(manager.store_upload(&id, "empty", b"").await.is_err());
    assert_eq!(fs::read_to_string(&path).await.unwrap(), original);
    let bad = manager.create_session().await.unwrap();
    manager.inner.state.set_session_file(&bad, outside.to_string_lossy().into_owned()).await.unwrap();
    assert!(manager.open_session(&bad, &[], &[]).await.is_err());
    assert!(manager.commands(&bad).await.is_err());
    assert!(manager.delete_session(&bad).await.is_err());
    assert_eq!(fs::read(&outside).await.unwrap(), b"private");
    manager.delete_session(&id).await.unwrap();
    assert!(!fs::try_exists(&path).await.unwrap());
    assert!(!fs::try_exists(&uploaded.path).await.unwrap());
    assert!(fs::try_exists(&artifact).await.unwrap());
    manager.shutdown().await;
    fs::remove_dir_all(root).await.unwrap();
}
