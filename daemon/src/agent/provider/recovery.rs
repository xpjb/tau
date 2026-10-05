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
        state.scope.is_object() && !state.image_started
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
mod tests {
    use super::*;
    fn item(id: &str) -> Value { json!({"type":"reasoning","id":id,"encrypted_content":"private-cipher","summary":[]}) }
    fn selected() -> SessionModel { SessionModel {provider:"openai-codex".into(),model_id:"gpt-6-astra".into()} }
    #[test]
    fn only_complete_encrypted_reasoning_is_recoverable() {
        let valid = item("rs_good"); assert_eq!(item_id(&valid),Some("rs_good"));
        for (key,value) in [("type",json!("function_call")),("status",json!("in_progress")),("id",json!("")),
            ("encrypted_content",json!(" ")), ("summary",Value::Null), ("summary",json!([{"type":"text","text":"x"}]))] {
            let mut invalid = valid.clone(); invalid[key] = value; assert_eq!(item_id(&invalid),None,"{key}");
        }
        let capture=Recovery::default(); Replay::new(&selected(),"account","https://fixture/codex",&[]).capture_into(&capture);
        capture.record(&valid); capture.record(&valid);
        assert_eq!(capture.snapshot().unwrap()["items"].as_array().unwrap().len(),1);
        assert!(capture.retry_safe()); capture.image_started(); assert!(!capture.retry_safe());
    }
    #[test]
    fn scope_dedup_and_rejection_follow_only_the_selected_branch() {
        let capture=Recovery::default(); Replay::new(&selected(),"account","https://fixture/codex/",&[]).capture_into(&capture);
        capture.record(&item("rs_one")); capture.record(&item("rs_two"));
        let message=json!({"role":"reasoning_recovery","recovery":capture.snapshot().unwrap()});
        for (account,endpoint) in [("other","https://fixture/codex"),("account","https://other/codex")] {
            assert!(Replay::new(&selected(),account,endpoint,&[]).items(&message).is_empty());
        }
        for model in [SessionModel {provider:"other".into(),..selected()},SessionModel {model_id:"other".into(),..selected()}] {
            assert!(Replay::new(&model,"account","https://fixture/codex",&[]).items(&message).is_empty());
        }
        let mut foreign_api=message.clone(); foreign_api["recovery"]["scope"]["api"]=json!("other");
        assert!(Replay::new(&selected(),"account","https://fixture/codex",&[]).items(&foreign_api).is_empty());
        let mut replay=Replay::new(&selected(),"account","https://fixture/codex",&[]);
        assert_eq!(replay.items(&message).len(),2); assert!(replay.items(&message).is_empty());
        let rejected=Recovery::default(); replay.capture_into(&rejected); rejected.reject(&["rs_one".into()]);
        let marker=json!({"role":"reasoning_recovery","recovery":rejected.snapshot().unwrap()});
        let completed=json!({"role":"assistant","codex_output":[item("rs_two")]});
        assert!(Replay::new(&selected(),"account","https://fixture/codex",&[marker.clone(),completed]).items(&message).is_empty());
        // A fork before the rejection retains the eligible original checkpoint.
        assert_eq!(Replay::new(&selected(),"account","https://fixture/codex",&[]).items(&message).len(),2);
        let mut next=Replay::new(&selected(),"account","https://fixture/codex",&[marker]);
        assert_eq!(next.items(&message),vec![item("rs_two")]);
        let again=Recovery::default(); next.capture_into(&again); again.record(&item("rs_two"));
        assert!(!again.advanced(),"Re-emitting an input checkpoint is not new progress");
    }
}
