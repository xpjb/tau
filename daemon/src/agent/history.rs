use std::collections::HashSet;
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
        if message["role"]=="assistant" && matches!(message["stopReason"].as_str(),Some("aborted"|"error")) {continue;}
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

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn dangling_private_calls_receive_unknown_outcomes_not_reexecution() {
        let native=json!({"role":"assistant","codex_output":[{"type":"reasoning","encrypted_content":"opaque"},{"type":"function_call","call_id":"call|item","name":"bash","arguments":"{}"}]});
        let entries=vec![json!({"type":"message","message":{"role":"assistant","stopReason":"toolUse","tauModelMessage":native}})];
        let model=SessionModel {provider:"p".into(),model_id:"m".into()};
        let result=messages(&entries,"system".into(),&model,Path::new("/unused")).await.unwrap();
        assert_eq!(result[1]["codex_output"][0]["encrypted_content"],"opaque");assert_eq!(result[2]["tool_call_id"],"call|item");
        assert!(result[2]["content"].as_str().unwrap().contains("unknown"));
        let mut completed=entries;completed.push(json!({"type":"message","message":{"role":"toolResult","toolCallId":"call|item","content":"done"}}));
        let result=messages(&completed,"system".into(),&model,Path::new("/unused")).await.unwrap();assert_eq!(result.len(),3);assert_eq!(result[2]["content"],"done");
    }
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
