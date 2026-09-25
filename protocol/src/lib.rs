use serde::{Deserialize, Serialize};

pub mod settings;
pub mod blocks;
mod transcript;
pub use transcript::*;

// Protocol 17 uses bounded control and one native data connection in both directions.
pub const PROTOCOL_VERSION: u32 = 18;
pub const MAX_CONTROL_BYTES: usize = 4096;
pub const MAX_REQUEST_BYTES: usize = 1024 * 1024;
pub const MAX_PROMPT_CHARS: usize = 256 * 1024;
pub const MAX_TITLE_CHARS: usize = 120;
pub const GENERAL_PROJECT_ID: &str = "general";
pub const MAX_PROJECT_NAME_CHARS: usize = 48;
pub const MAX_PROJECT_PROMPT_CHARS: usize = 64 * 1024;
pub fn general_project_id() -> String { GENERAL_PROJECT_ID.into() }

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Project {
    pub id: String,
    pub name: String,
    pub prompt: String,
    pub revision: u64,
}
impl Project {
    pub fn general() -> Self {
        Self { id: general_project_id(), name: "General".into(), prompt: String::new(), revision: 0 }
    }
}
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeleteProjectMode { MoveToGeneral, DeleteChats }

pub const MAX_CRASH_BYTES: usize = 24 * 1024;
pub const MAX_UPLOAD_BYTES: usize = 50_000_000;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ClientRequest {
    pub id: String,
    #[serde(flatten)]
    pub command: ClientCommand,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ClientCommand {
    ConnectBlocks { node_id: String },
    /// A complete ClientRequest uploaded through the native data connection.
    Input { content: blocks::ContentRef },
    GetSession { session_id: String },
    GetReceipts { session_id: String, requests: Vec<String> },
    GetOperation { operation_id: String },
    ListSessions,
    ReviewRestore {session_id:String},
    ListPage {catalog_id:String,projects:bool,after:Option<String>,revision:u64},
    CreateProject { project_id: String, name: String, prompt: String },
    UpdateProject { project_id: String, revision: u64, name: String, prompt: String },
    DeleteProject { project_id: String, revision: u64, mode: DeleteProjectMode },
    MoveSession { session_id: String, project_id: String },
    GetSettings,
    SetSettings { revision: u64, settings: Box<settings::Settings> },
    RefreshModelCatalog { provider: String },
    CreateSession {
        #[serde(default = "general_project_id")]
        project_id: String,
        #[serde(default)]
        keep_session_id: Option<String>,
    },
    #[serde(skip)] // Internal legacy adapter only; no wire route.
    OpenSession {
        session_id: String,
        #[serde(default)]
        requests: Vec<String>,
    },
    #[serde(skip)]
    GetHistory {
        session_id: String,
        generation: String,
        before: u64,
    },
    GetCommands {
        session_id: String,
    },
    Prompt {
        session_id: String,
        text: String,
    },
    QueueControl {
        session_id: String,
        generation: String,
        operation: QueueOperation,
    },
    Abort {
        session_id: String,
    },
    CloseSession {
        session_id: String,
    },
    DeleteSession {
        session_id: String,
    },
    RenameSession {
        session_id: String,
        title: String,
    },
    ForkSession {
        session_id: String,
        entry_id: String,
    },
    CloneSession {
        session_id: String,
    },
}

impl ClientCommand {
    pub fn journalled_control(&self)->bool {
        matches!(self,Self::ReviewRestore {..}|Self::CreateSession {..}|Self::CreateProject {..}|Self::UpdateProject {..}|Self::DeleteProject {..}
            |Self::MoveSession {..}|Self::SetSettings {..}|Self::CloseSession {..}|Self::DeleteSession {..}
            |Self::RenameSession {..}|Self::ForkSession {..}|Self::CloneSession {..})
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum QueueOperation {
    Edit {
        request_id: String,
        revision: u64,
        text: String,
    },
    Delete {
        request_id: String,
        revision: u64,
    },
    Prefix {
        run_id: Option<String>,
        requests: Vec<QueueRef>,
        boundary: String,
    },
    Pause {
        run_id: Option<String>,
        boundary: String,
    },
    Resume {
        run_id: Option<String>,
    },
    Cancel {
        control_id: String,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
pub enum ServerMessage {
    BlockConnection { offer: blocks::BulkOffer },
    /// Large descriptors travel over native data, never as a WebSocket body.
    Data { content: blocks::ContentRef, key: String, session_id: Option<String>, reports: Vec<OperationReceipt>, operation_id: Option<String>, route:Option<String> },
    Receipts { session_id: String, reports: Vec<OperationReceipt> },
    Accepted { request_id: String },
    Operation { operation_id: String, registered: bool, response: Option<Box<ServerMessage>> },
    Hello {
        protocol_version: u32,
        daemon_version: String,
        #[serde(default)] lineage:Option<String>,
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
    Settings { request_id: String, settings: Box<settings::Settings> },
    Commands {
        session_id: String,
        commands: Vec<SlashCommand>,
    },
    Notice { session_id: String, message: String },
    SessionPage {catalog_id:String,revision:u64,after:Option<String>,next:Option<String>,sessions:Vec<SessionSummary>,states:std::collections::BTreeMap<String,u64>},
    ProjectPage {catalog_id:String,revision:u64,after:Option<String>,next:Option<String>,projects:Vec<Project>},
    Projects { projects: Vec<Project> },
    Sessions {
        sessions: Vec<SessionSummary>,
    },
    #[serde(skip)] // Rendering/provider adapter, not a control message.
    TranscriptSnapshot {
        session_id: String,
        snapshot: TranscriptSnapshot,
    },
    #[serde(skip)] // Rendering/provider adapter, not a control message.
    TranscriptPage {
        request_id: String,
        session_id: String,
        generation: String,
        cursor: u64,
        page: HistoryPage,
    },
    #[serde(skip)] // Rendering/provider adapter, not a control message.
    TranscriptUpdate {
        session_id: String,
        generation: String,
        sequence: u64,
        change: TranscriptChange,
    },
    SessionState {
        session_id: String,
        #[serde(default)] revision:u64,
        #[serde(default)] restore_review:Option<bool>,
        status: SessionStatus,
        context_usage: Option<ContextUsage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    ResyncRequired {
        #[serde(skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
    },
}

impl ServerMessage {
    /// Only superseded replicated state may be coalesced. Response IDs retain
    /// distinct delivery; their durable acceptance is not a display cursor.
    pub fn replication_key(&self) -> Option<String> {
        match self {
            Self::Sessions {..}=>Some("sessions".into()),Self::Projects {..}=>Some("projects".into()),
            Self::SessionState {session_id,..}=>Some(format!("state:{session_id}")),
            Self::Commands {session_id,..}=>Some(format!("commands:{session_id}")),
            Self::Settings {..}=>Some("settings".into()),
            Self::Data {key,..}=>Some(key.clone()),
            _=>None,
        }
    }

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

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStatus {
    #[default]
    Sleeping,
    Idle,
    Running,
    Error,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsage {
    /// Last provider-reported turn total. This is not a live tokenizer count.
    pub tokens: Option<u64>,
    pub context_window: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSummary {
    pub id: String,
    #[serde(default = "general_project_id")]
    pub project_id: String,
    pub title: String,
    pub starter: bool,
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
    Accepted,
    Submitted,
    Queued,
    Handled,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFile {
    pub name: String,
    pub path: String,
    pub size: u64,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_range: Option<CrashRange>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub causes: Vec<CrashCause>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashCause {
    pub exception_class: String,
    pub stack: Vec<CrashFrame>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection_range: Option<CrashRange>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashRange {
    pub start: i32,
    pub end: i32,
    pub text_length: i32,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CrashFrame {
    pub class_name: String,
    pub method_name: String,
    pub file_name: Option<String>,
    pub line_number: i32,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionModel {
    pub provider: String,
    pub model_id: String,
}

impl std::str::FromStr for SessionModel {
    type Err = &'static str;
    fn from_str(slug: &str) -> Result<Self, Self::Err> {
        let (provider, model) = slug.split_once('/').ok_or("Use provider/model")?;
        if provider.is_empty() || model.is_empty() || slug.len() > 240
            || slug.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err("Use provider/model without whitespace (up to 240 bytes)");
        }
        Ok(Self { provider: provider.into(), model_id: model.into() })
    }
}

/// Durable intent outcome, independent of display blocks and transcript windows.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all="camelCase")]
pub struct OperationReceipt { pub id:String, pub accepted:bool, pub complete:bool, pub error:Option<String>, pub notice:Option<String> }
