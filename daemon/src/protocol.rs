use serde::{Deserialize, Serialize};

use crate::state::SessionModel;

pub const PROTOCOL_VERSION: u32 = 1;
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
    Abort { session_id: String },
    CloseSession { session_id: String },
    DeleteSession { session_id: String },
    RenameSession { session_id: String, title: String },
    ForkSession { session_id: String, entry_id: String },
    CloneSession { session_id: String },
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
        command_handled: Option<bool>,
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
    History {
        session_id: String,
        messages: Vec<ChatMessage>,
    },
    SessionState {
        session_id: String,
        status: SessionStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    StreamReset {
        session_id: String,
    },
    StreamDelta {
        session_id: String,
        delta: String,
    },
    StreamDetailsDelta {
        session_id: String,
        delta: String,
    },
    StreamEnd {
        session_id: String,
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
            command_handled: None,
            notice: None,
            error: None,
        }
    }

    pub fn prompt_success(
        request_id: String,
        session_id: String,
        command_handled: bool,
        notice: Option<String>,
    ) -> Self {
        Self::Response {
            request_id,
            ok: true,
            session_id: Some(session_id),
            draft: None,
            command_handled: Some(command_handled),
            notice,
            error: None,
        }
    }

    pub fn failure(request_id: String, error: impl Into<String>) -> Self {
        Self::Response {
            request_id,
            ok: false,
            session_id: None,
            draft: None,
            command_handled: None,
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

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    pub title: String,
    pub status: SessionStatus,
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<SessionModel>,
    pub parent_id: Option<String>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatDetailKind {
    Thinking,
    Tool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDetail {
    pub kind: ChatDetailKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub arguments: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<String>,
    pub is_error: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatMessage {
    pub entry_id: String,
    pub role: ChatRole,
    pub text: String,
    pub timestamp_ms: Option<u64>,
    pub has_details: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub details: Vec<ChatDetail>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attachment: Option<ChatAttachment>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatDetails {
    pub details: Vec<ChatDetail>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatAttachment {
    pub kind: AttachmentKind,
    pub file_name: String,
    pub caption: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentKind {
    Image,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatRole {
    User,
    Assistant,
    System,
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
