//! Provider-private recovery state for one request, never a display message.
use std::collections::HashSet;
use std::sync::Mutex;
use serde_json::{Value, json};
use crate::state::SessionModel;

pub fn item_id(item: &Value) -> Option<&str> {
    if item["type"] != "reasoning"
        || item.get("status").is_some_and(|status| status != "completed")
        || !item["encrypted_content"].as_str().is_some_and(|text| !text.trim().is_empty())
        || !item["summary"].as_array().is_some_and(|parts| parts.iter().all(|part|
            part["type"] == "summary_text" && part["text"].is_string())) { return None; }
    item["id"].as_str().filter(|id| !id.is_empty())
}

pub struct Replay {
    scope: Value,
    seen: HashSet<String>,
    pub used: Vec<String>,
}
impl Replay {
    pub fn new(selected: &SessionModel, account: &str, endpoint: &str, messages: &[Value]) -> Self {
        let scope = json!({"version":1,"provider":selected.provider,"api":"codex","model":selected.model_id,
            "accountId":account,"baseUrl":endpoint.trim_end_matches('/')});
        // Rejections are branch-local and durable. Completed normal turns also
        // own their item IDs; do not replay the same reasoning a second time.
        let seen = messages.iter().flat_map(|message| {
            let rejected = (message["role"] == "reasoning_recovery" && message["recovery"]["scope"] == scope)
                .then(|| message["recovery"]["rejectedIds"].as_array()).flatten().into_iter().flatten().filter_map(Value::as_str);
            let completed = message["codex_output"].as_array().into_iter().flatten()
                .filter(|item| item["type"] == "reasoning").filter_map(|item| item["id"].as_str());
            rejected.chain(completed).map(str::to_owned)
        }).collect();
        Self { scope, seen, used:Vec::new() }
    }
    pub fn items(&mut self, message: &Value) -> Vec<Value> {
        if message["recovery"]["scope"] != self.scope { return Vec::new(); }
        message["recovery"]["items"].as_array().into_iter().flatten().filter_map(|item| {
            let id = item_id(item)?;
            if !self.seen.insert(id.to_owned()) { return None; }
            self.used.push(id.to_owned()); Some(item.clone())
        }).collect()
    }
    pub fn capture_into(&self, recovery: &Recovery) {
        *recovery.0.lock().unwrap() = Capture {scope:self.scope.clone(), known:self.seen.clone(), ..Capture::default()};
    }
}

#[derive(Default)]
struct Capture {
    scope: Value,
    known: HashSet<String>,
    items: Vec<Value>,
    rejected: Vec<String>,
    image_started: bool,
}
#[derive(Default)]
pub struct Recovery(Mutex<Capture>);
impl Recovery {
    pub fn record(&self, item: &Value) {
        let Some(id) = item_id(item) else { return; };
        let mut state = self.0.lock().unwrap();
        if state.known.insert(id.to_owned()) { state.items.push(item.clone()); }
    }
    pub fn image_started(&self) { self.0.lock().unwrap().image_started = true; }
    pub fn reject(&self, ids: &[String]) { self.0.lock().unwrap().rejected = ids.to_vec(); }
    pub fn retry_safe(&self) -> bool {
        let state = self.0.lock().unwrap();
        // Scope controls encrypted-checkpoint replay, not transport retry safety.
        // Chat-completions has no Codex scope and still has no executed local tools.
        !state.image_started
    }
    pub fn advanced(&self) -> bool { !self.0.lock().unwrap().items.is_empty() }
    pub fn snapshot(&self) -> Option<Value> {
        let state = self.0.lock().unwrap();
        if state.items.is_empty() && state.rejected.is_empty() { return None; }
        Some(json!({"scope":state.scope,"items":state.items,"rejectedIds":state.rejected}))
    }
}

pub fn reasoning_rejected(error: &Value) -> bool {
    let code = error["code"].as_str().or_else(|| error["metadata"]["error_type"].as_str()).unwrap_or_default();
    let message = error["message"].as_str().unwrap_or_default().to_ascii_lowercase();
    matches!(code, "invalid_encrypted_content" | "invalid_reasoning_signature")
        || message.contains("encrypted_content") || message.contains("reasoning item")
}

#[cfg(test)]
#[path = "../../../tests/unit/agent/provider/recovery.rs"]
mod tests;
