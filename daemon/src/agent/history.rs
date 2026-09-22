use std::collections::HashSet;
use std::path::Path;
use anyhow::{Context, Result};
use serde_json::{Value, json};
use crate::state::SessionModel;

    pub async fn messages(entries: &[Value], system: String, selected: &SessionModel, attachment_root: &Path) -> Result<Vec<Value>> {
    let mut output = vec![json!({"role":"system", "content":system})];
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
                output.push(json!({"role":"tool","tool_call_id":message["toolCallId"].as_str().unwrap_or_default().split('|').next().unwrap_or_default(),"content":text}));
                if !images.is_empty() { output.push(json!({"role":"user","content":images})); }
            }
            Some("bashExecution") => output.push(json!({"role":"user","content":format!("Shell output:\n{}", message["output"].as_str().unwrap_or_default())})),
            _ => {},
        }
    }
    // A daemon crash must not cause a tool call to be executed twice. Missing results are explicit.
    let completed = output.iter().filter_map(|message| message["tool_call_id"].as_str()).map(str::to_owned).collect::<HashSet<_>>();
    let mut repaired = Vec::new();
    for message in output {
        let missing = message["tool_calls"].as_array().into_iter().flatten().filter_map(|call| call["id"].as_str())
            .filter(|id| !completed.contains(*id)).map(str::to_owned).collect::<Vec<_>>();
        repaired.push(message);
        for id in missing { repaired.push(json!({"role":"tool","tool_call_id":id,"content":"Execution was interrupted. Its effects are unknown; inspect the filesystem before retrying."})); }
    }
    Ok(repaired)
}


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
