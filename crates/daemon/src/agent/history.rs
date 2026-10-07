use std::collections::{HashMap, HashSet};
use std::path::Path;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use crate::state::SessionModel;

    pub async fn messages(entries: &[Value], system: String, selected: &SessionModel, attachment_root: &Path) -> Result<Vec<Value>> {
    let mut output = vec![json!({"role":"system", "content":system})];
    let mut checked=0;let mut bytes=0;
    let mut start = 0;
    for entry in entries.iter().rev().filter(|entry| entry["type"] == "compaction") {
        let native = entry.pointer("/details/kind").and_then(Value::as_str) == Some("codex-native-compaction");
        if native && (entry["details"]["model"] != selected.model_id || entry["details"]["provider"] != selected.provider) { continue; }
        let first = entry["firstKeptEntryId"].as_str().context("Compaction has no retained boundary")?;
        start = entries.iter().position(|entry| entry["id"] == first).context("Compaction boundary is missing")?;
        if native {
            output.push(json!({"role":"checkpoint","model":entry["details"]["model"],"accountId":entry["details"]["accountId"],
                "baseUrl":entry["details"]["baseUrl"],"item":entry["details"]["item"]}));
        } else { output.push(json!({"role":"user","content":format!("Previous context summary:\n{}", entry["summary"].as_str().unwrap_or_default())})); }
        break;
    }
    for entry in &entries[start..] {
        checked=check_size(&output,checked,&mut bytes)?;
        if entry["type"] == "tau_attachment" {
            let request = crate::transcript::attachment_request(entry).context("Invalid generated attachment record")?;
            let mut parts = vec![json!({"type":"text","text":"The assistant generated this image in the preceding response. It is a reference image, not a new user request."})];
            match crate::attachments::image_reference(attachment_root, &request).await {
                Ok(image) => parts.push(image),
                Err(_) => parts.push(json!({"type":"text","text":"The original generated image file is no longer available. Do not assume its contents or silently regenerate it."})),
            }
            output.push(json!({"role":"user","content":parts})); continue;
        }
        if entry["type"] == "custom_message" {
            output.push(json!({"role":"user","content":entry["content"].as_str().unwrap_or_default()})); continue;
        }
        if entry["type"] != "message" { continue; }
        let message = &entry["message"];
        if message["role"]=="assistant" && matches!(message["stopReason"].as_str(),Some("aborted"|"error")) {
            if let Some(recovery) = message.get("tauReasoningRecovery") {
                output.push(json!({"role":"reasoning_recovery","recovery":recovery}));
            }
            continue;
        }
        if let Some(native) = message.get("tauModelMessage") { output.push(native.clone()); continue; }
        let content = &message["content"];
        let text = content.as_str().map(str::to_owned).unwrap_or_else(|| content.as_array().into_iter().flatten()
            .filter(|part| part["type"] == "text").filter_map(|part| part["text"].as_str()).collect::<Vec<_>>().join("\n"));
        let images = content.as_array().into_iter().flatten().filter(|part| part["type"] == "image").map(|part|
            json!({"type":"image_url", "image_url":{"url":format!("data:{};base64,{}", part["mimeType"].as_str().unwrap_or("image/png"), part["data"].as_str().unwrap_or_default())}})).collect::<Vec<_>>();
        match message["role"].as_str() {
            Some("user") => {
                if images.is_empty() { output.push(json!({"role":"user","content":text})); }
                else { let mut parts = vec![json!({"type":"text","text":text})]; parts.extend(images); output.push(json!({"role":"user","content":parts})); }
            }
            Some("assistant") => {
                if matches!(message["stopReason"].as_str(), Some("aborted" | "error")) { continue; }
                let calls = content.as_array().into_iter().flatten().filter(|part| part["type"] == "toolCall").map(|part|
                    json!({"id":part["id"].as_str().unwrap_or_default().split('|').next().unwrap_or_default(),"type":"function","function":{"name":part["name"],"arguments":part["arguments"].to_string()}})).collect::<Vec<_>>();
                let mut converted = json!({"role":"assistant","content":text});
                if !calls.is_empty() { converted["tool_calls"] = json!(calls); }
                output.push(converted);
            }
            Some("toolResult") => {
                let raw=message["toolCallId"].as_str().unwrap_or_default();let short=raw.split('|').next().unwrap_or_default();
                let id=output.iter().rev().flat_map(call_ids).find(|id|id==raw || id.split('|').next()==Some(short)).unwrap_or_else(||short.to_owned());
                output.push(json!({"role":"tool","tool_call_id":id,"content":text}));
                if !images.is_empty() { output.push(json!({"role":"user","content":images})); }
            }
            Some("bashExecution") => output.push(json!({"role":"user","content":format!("Shell output:\n{}", message["output"].as_str().unwrap_or_default())})),
            _ => {},
        }
    }
    // A daemon crash must not cause a tool call to be executed twice. Missing results are explicit.
    check_size(&output,checked,&mut bytes)?;
    let mut completed=HashSet::new();let mut missing=vec![vec![];output.len()];
    for (index,message) in output.iter().enumerate().rev() {
        missing[index]=call_ids(message).into_iter().filter(|id|!completed.contains(id)).collect();
        if message["role"]=="assistant" || message["role"]=="checkpoint" {completed.clear();}
        if let Some(id)=message["tool_call_id"].as_str() {completed.insert(id.to_owned());}
    }
    let mut repaired = Vec::new();
    for (index,message) in output.into_iter().enumerate() {
        repaired.push(message);
        for id in &missing[index] {
            repaired.push(json!({"role":"tool","tool_call_id":id,"content":"Execution was interrupted. Its effects are unknown; inspect the filesystem and obtain confirmation before retrying. Do not automatically repeat paid or external effects."}));
        }
    }
    check_size(&repaired,0,&mut 0)?;
    Ok(repaired)
}


fn check_size(messages:&[Value],from:usize,total:&mut usize)->Result<usize> {
    struct Count(usize);
    impl std::io::Write for Count {fn write(&mut self,bytes:&[u8])->std::io::Result<usize> {self.0=self.0.saturating_add(bytes.len());Ok(bytes.len())}fn flush(&mut self)->std::io::Result<()> {Ok(())}}
    for message in &messages[from..] {
        let mut count=Count(0);serde_json::to_writer(&mut count,message)?;*total=total.saturating_add(count.0);
        anyhow::ensure!(*total<=128*1024*1024,"Prepared provider context exceeds 128 MiB, including image references; export/fork a smaller conversation");
    }Ok(messages.len())
}

fn call_ids(message:&Value)->Vec<String> {
    let mut seen=HashSet::new();
    message["tool_calls"].as_array().into_iter().flatten().filter_map(|call|call["id"].as_str())
        .chain(message["codex_output"].as_array().into_iter().flatten().filter(|item|item["type"]=="function_call").filter_map(|item|item["call_id"].as_str()))
        .filter(|id|seen.insert(*id)).map(str::to_owned).collect()
}


/// Prefer whole user turns within the retention budget, but a long single task
/// must also be compactable between assistant/tool exchanges. Only stored
/// results constrain a cut: missing results already become explicit unknown
/// outcomes in `messages`, and must not poison every later boundary forever.
pub fn compaction_cut(entries: &[Value], keep_tokens: u64) -> Result<usize> {
    let mut pending = HashMap::new();
    let mut last_result = vec![0; entries.len()];
    for (index, entry) in entries.iter().enumerate() {
        if entry["type"] != "message" { continue; }
        let message = &entry["message"];
        match message["role"].as_str() {
            Some("assistant") if !matches!(message["stopReason"].as_str(), Some("error" | "aborted")) => {
                let calls = message.get("tauModelMessage").map(call_ids).unwrap_or_else(|| {
                    message["content"].as_array().into_iter().flatten().filter(|part| part["type"] == "toolCall")
                        .filter_map(|part| part["id"].as_str().map(str::to_owned)).collect()
                });
                for call in calls { pending.insert(call, index); }
            }
            Some("toolResult") => {
                if let Some(id) = message["toolCallId"].as_str() {
                    // Imported history may use call|item on only one side. An
                    // ambiguous alias cannot settle two distinct native calls.
                    let calls = if pending.contains_key(id) { vec![id.to_owned()] } else {
                        pending.keys().filter(|call| call.split('|').next() == id.split('|').next()).cloned().collect()
                    };
                    for call in &calls { last_result[pending[call]] = index; }
                    if calls.len() == 1 { pending.remove(&calls[0]); }
                }
            }
            _ => {},
        }
    }
    let mut boundaries = Vec::new();
    let mut through = 0;
    let mut size = 0u64;
    let mut have_history = false;
    for (index, entry) in entries.iter().enumerate() {
        let message = &entry["message"];
        let role = message["role"].as_str();
        if entry["type"] == "message" {
            if role == Some("assistant") && matches!(message["stopReason"].as_str(), Some("error" | "aborted")) { continue; }
            if have_history && index > through && matches!(role, Some("user" | "assistant")) {
                boundaries.push((index, size, role == Some("user")));
            }
            through = through.max(last_result[index]);
        }
        if matches!(entry["type"].as_str(), Some("message" | "tau_attachment" | "custom_message")) {
            have_history = true;
            size = size.saturating_add(estimate_tokens(if entry["type"] == "custom_message" { &entry["content"] } else { message }));
        }
    }
    boundaries.iter().find(|(_,before,user)| *user && size.saturating_sub(*before) <= keep_tokens)
        .or_else(|| boundaries.iter().find(|(_,before,_)| size.saturating_sub(*before) <= keep_tokens))
        .or_else(|| boundaries.last()).map(|(index,_,_)| *index)
        .context("Not enough completed exchanges to compact safely; reduce the input or fork an earlier turn")
}

#[cfg(test)]
#[path = "../../tests/unit/agent/history.rs"]
mod tests;

// Images/checkpoints are opaque provider inputs, not millions of base64 text tokens.
pub fn estimate_tokens(value: &Value) -> u64 {
    match value {
        Value::String(text) => (text.len() as u64).div_ceil(4),
        Value::Array(items) => items.iter().map(estimate_tokens).sum(),
        Value::Object(fields) => {
            if matches!(fields.get("type").and_then(Value::as_str), Some("image" | "image_url" | "input_image" | "compaction")) { return 2048; }
            fields.iter().filter(|(key,_)| !matches!(key.as_str(), "encrypted_content" | "tauModelMessage" | "codex_output" | "usage"))
                .map(|(_,value)| estimate_tokens(value)).sum()
        }
        _ => 0,
    }
}
