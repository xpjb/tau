use serde::{Deserialize, Serialize};

use crate::state::SessionModel;
use crate::transcript::{QueueRef, TranscriptChange, TranscriptSnapshot};

pub const PROTOCOL_VERSION: u32 = 3;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_PROMPT_CHARS: usize = 256 * 1024;
pub const MAX_TITLE_CHARS: usize = 120;
pub const MAX_CRASH_BYTES: usize = 24 * 1024;
pub const MAX_UPLOAD_BYTES: usize = 50_000_000;

#[derive(Debug, Deserialize)]
pub struct ClientRequest {
    pub id: String,
    #[serde(flatten)]
    pub command: ClientCommand,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ClientCommand {
    ListSessions,
    CreateSession,
    OpenSession { session_id: String },
    GetCommands { session_id: String },
    Prompt { session_id: String, text: String },
    ExtensionUiResponse {
        session_id: String,
        request_id: String,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        confirmed: Option<bool>,
        #[serde(default)]
        cancelled: bool,
    },
    QueueControl { session_id: String, generation: String, operation: QueueOperation },
    Abort { session_id: String },
    CloseSession { session_id: String },
    DeleteSession { session_id: String },
    RenameSession { session_id: String, title: String },
    ForkSession { session_id: String, entry_id: String },
    CloneSession { session_id: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum QueueOperation {
    Edit { request_id: String, revision: u64, text: String },
    Delete { request_id: String, revision: u64 },
    Prefix { run_id: Option<String>, requests: Vec<QueueRef>, boundary: String },
    Pause { run_id: Option<String>, boundary: String },
    Resume { run_id: Option<String> },
    Cancel { control_id: String },
}

#[derive(Clone, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", rename_all_fields = "camelCase")]
pub enum ServerMessage {
    Hello {
        protocol_version: u32,
        daemon_version: &'static str,
    },
    Response {
        request_id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        draft: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        disposition: Option<PromptDisposition>,
        uncertain: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        outcome: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        notice: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    Commands {
        session_id: String,
        commands: Vec<SlashCommand>,
    },
    ExtensionUi {
        session_id: String,
        request: Box<ExtensionUiRequest>,
    },
    ExtensionError {
        session_id: String,
        error: String,
    },
    Sessions {
        sessions: Vec<SessionSummary>,
    },
    TranscriptSnapshot {
        session_id: String,
        snapshot: TranscriptSnapshot,
    },
    TranscriptUpdate {
        session_id: String,
        generation: String,
        sequence: u64,
        change: TranscriptChange,
    },
    SessionState {
        session_id: String,
        status: SessionStatus,
        context_usage: Option<ContextUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    ResyncRequired,
}

impl ServerMessage {
    pub fn success(request_id: String, session_id: Option<String>, draft: Option<String>) -> Self {
        Self::Response {
            request_id,
            ok: true,
            session_id,
            draft,
            disposition: None,
            uncertain: false,
            outcome: None,
            notice: None,
            error: None,
        }
    }

    pub fn prompt_success(
        request_id: String,
        session_id: String,
        disposition: PromptDisposition,
        notice: Option<String>,
    ) -> Self {
        Self::Response {
            request_id,
            ok: true,
            session_id: Some(session_id),
            draft: None,
            disposition: Some(disposition),
            uncertain: false,
            outcome: None,
            notice,
            error: None,
        }
    }

    pub fn command_failure(request_id: String, error: anyhow::Error) -> Self {
        let mut response = Self::failure(request_id, error.to_string());
        if let Self::Response { uncertain, .. } = &mut response {
            *uncertain = error.is::<crate::pi::UnconfirmedCommand>();
        }
        response
    }

    pub fn failure(request_id: String, error: impl Into<String>) -> Self {
        Self::Response {
            request_id,
            ok: false,
            session_id: None,
            draft: None,
            disposition: None,
            uncertain: false,
            outcome: None,
            notice: None,
            error: Some(error.into()),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SlashCommandSource {
    Extension,
    Prompt,
    Skill,
    Builtin,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommandArgument {
    pub value: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub source: SlashCommandSource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<SlashCommandArgument>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionUiRequest {
    pub id: String,
    pub method: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notify_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_key: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub widget_lines: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub widget_placement: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Sleeping,
    Starting,
    Idle,
    Running,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    pub tokens: Option<u64>,
    pub context_window: u64,
}

impl ContextUsage {
    pub fn from_pi(data: &serde_json::Value) -> Option<Self> {
        let usage: Self = serde_json::from_value(data.get("contextUsage")?.clone()).ok()?;
        (usage.context_window > 0 && usage.context_window <= 9_007_199_254_740_991
            && usage.tokens.is_none_or(|tokens| tokens <= 9_007_199_254_740_991)).then_some(usage)
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub status: SessionStatus,
    pub detail: Option<String>,
    pub context_usage: Option<ContextUsage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<SessionModel>,
    pub parent_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptDisposition {
    Submitted,
    Queued,
    Handled,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub kind: AttachmentKind,
    pub file_name: String,
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashReport {
    pub schema: u32,
    pub report_id: String,
    pub platform: String,
    pub app_version: String,
    pub os_version: String,
    pub thread: String,
    pub exception_class: String,
    pub stack: Vec<CrashFrame>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashFrame {
    pub class_name: String,
    pub method_name: String,
    pub file_name: Option<String>,
    pub line_number: i32,
}
