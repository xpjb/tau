use anyhow::{bail, Context as _, Result};
use serde_json::{json, Value};
use super::{Decoder, Stream};
use std::collections::HashSet;
pub struct Codex;
impl Decoder for Codex {
    fn decode(&self, event: &Value, stream: &mut Stream<'_>) -> Result<()> {
        let kind = event.get("type").and_then(Value::as_str)
            .context("Codex event has no type or ended without response.completed")?;
        if let Some(response) = event.get("response") {
            if let Some(id) = response.get("id").and_then(Value::as_str) {
                stream.id = Some(id.into());
            }
            if let Some(model) = response.get("model").and_then(Value::as_str) {
                stream.model = Some(model.into());
            }
        }
        match kind {
            "response.output_text.delta" | "response.refusal.delta"
            | "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let text = event.get("delta").and_then(Value::as_str)
                    .context("Codex text delta was not text")?;
                let key = if matches!(kind, "response.output_text.delta" | "response.refusal.delta") {
                    "content"
                } else {
                    "reasoning"
                };
                let Value::String(assembled) = stream.message.entry(key)
                    .or_insert_with(|| Value::String(String::new()))
                else {
                    bail!("assembled Codex text had the wrong type");
                };
                assembled.push_str(text);
                if !text.is_empty() {
                    stream.progress_events += 1;
                }
            }
            "response.web_search_call.in_progress" | "response.web_search_call.searching"
            | "response.web_search_call.completed" => stream.progress_events += 1,
            "response.image_generation_call.in_progress" | "response.image_generation_call.generating"
            | "response.image_generation_call.completed" | "response.image_generation_call.partial_image" => {
                stream.progress_events += 1;
            }
            "response.output_item.added" | "response.output_item.done" => {
                let item = event.get("item").context("Codex output event has no item")?;
                if item.get("type").and_then(Value::as_str) == Some("image_generation_call") {
                    stream.progress_events += 1;
                }
                let index = event.get("output_index").and_then(Value::as_u64)
                    .context("Codex item has no output index")? as usize;
                let completed = stream.output_items.entry(index).or_insert(None);
                if kind == "response.output_item.done" {
                    if completed.is_some() {
                        bail!("Codex returned a duplicate completed output index");
                    }
                    *completed = Some(item.clone());
                    stream.progress_events += 1;
                } else if item.get("type").and_then(Value::as_str) == Some("function_call") {
                    stream.tool_calls.insert(index, json!({
                        "id": item["call_id"], "type": "function",
                        "function": {"name": item["name"], "arguments": ""}
                    }).as_object().unwrap().clone());
                    stream.progress_events += 1;
                }
            }
            "response.function_call_arguments.delta" => {
                let index = event.get("output_index").and_then(Value::as_u64)
                    .context("Codex arguments have no output index")? as usize;
                let delta = event.get("delta").and_then(Value::as_str)
                    .context("Codex argument delta was not text")?;
                let call = stream.tool_calls.get_mut(&index).context("Codex arguments arrived before their call")?;
                let arguments = call.get_mut("function").and_then(|function| function.get_mut("arguments"))
                    .context("Codex call has no arguments")?;
                let Value::String(arguments) = arguments else {
                    bail!("Codex arguments were not text");
                };
                arguments.push_str(delta);
                if !delta.is_empty() {
                    stream.progress_events += 1;
                }
            }
            "error" | "response.failed" => {
                let error = event.get("error")
                    .or_else(|| event.get("response").and_then(|response| response.get("error")))
                    .unwrap_or(event);
                let code = error.get("code").cloned().unwrap_or(Value::Null);
                let status = match code.as_str() {
                    Some("server_error" | "internal_error" | "internal_server_error") => json!(500),
                    _ => code.clone(),
                };
                stream.error = Some(json!({
                    "code": status, "message": error.get("message"),
                    "metadata": {"error_type": code}
                }));
                stream.finish_reason = Some("error".into());
            }
            "response.completed" | "response.done" | "response.incomplete" => {
                let response = event.get("response").context("Codex completion has no response")?;
                let status = response.get("status").and_then(Value::as_str)
                    .context("Codex completion has no status")?;
                if kind == "response.incomplete" || status != "completed" {
                    stream.done = true;
                    stream.finish_reason = Some("incomplete".into());
                    return Ok(());
                }
                let mut output = match response.get("output") {
                    Some(Value::Array(items)) => items.clone(),
                    None => Vec::new(),
                    _ => bail!("Codex completion has invalid output items"),
                };
                if output.is_empty() {
                    for completed in std::mem::take(&mut stream.output_items).into_values() {
                        let Some(item) = completed else {
                            stream.done = true;
                            stream.finish_reason = Some("incomplete".into());
                            return Ok(());
                        };
                        output.push(item);
                    }
                } else {
                    stream.output_items.clear();
                }
                let mut content = String::new();
                let mut annotations = Vec::new();
                let mut calls = std::collections::BTreeMap::new();
                let mut call_ids = HashSet::new();
                for (index, item) in output.iter_mut().enumerate() {
                    if item.get("status").and_then(Value::as_str)
                        .is_some_and(|status| status != "completed")
                    {
                        stream.done = true;
                        stream.finish_reason = Some("incomplete".into());
                        return Ok(());
                    }
                    match item.get("type").and_then(Value::as_str) {
                        Some("message") => {
                            for part in item.get("content").and_then(Value::as_array)
                                .context("Codex message has no content")?
                            {
                                match part.get("type").and_then(Value::as_str) {
                                    Some("output_text") => {
                                        content.push_str(part.get("text").and_then(Value::as_str)
                                            .context("Codex output was not text")?);
                                        if let Some(items) = part.get("annotations").and_then(Value::as_array) {
                                            annotations.extend(items.iter().cloned());
                                        }
                                    }
                                    Some("refusal") => content.push_str(part.get("refusal").and_then(Value::as_str)
                                        .context("Codex refusal was not text")?),
                                    _ => bail!("unsupported Codex output content"),
                                }
                            }
                        }
                        Some("function_call") => {
                            let id = item.get("call_id").and_then(Value::as_str)
                                .filter(|id| !id.is_empty()).context("Codex function call has no ID")?;
                            if !call_ids.insert(id) {
                                bail!("Codex returned duplicate function call IDs");
                            }
                            let name = item.get("name").and_then(Value::as_str)
                                .filter(|name| !name.is_empty()).context("Codex function call has no name")?;
                            let arguments = item.get("arguments").and_then(Value::as_str)
                                .context("Codex function call has no argument string")?;
                            calls.insert(index, json!({"id": id, "type": "function",
                                "function": {"name": name, "arguments": arguments}}).as_object().unwrap().clone());
                        }
                        Some("web_search_call") => {},
                        Some("reasoning" | "compaction") => {}
                        _ => bail!("unsupported Codex output item"),
                    }
                }
                output.retain(|item| !item.is_null());
                stream.tool_calls = calls;
                stream.message.insert("content".into(), Value::String(content));
                stream.message.insert("annotations".into(), Value::Array(annotations));
                stream.message.insert("codex_output".into(), Value::Array(output));
                stream.finish_reason = Some(if stream.tool_calls.is_empty() { "stop" } else { "tool_calls" }.into());
                if let Some(usage) = response.get("usage").filter(|usage| !usage.is_null()) {
                    let prompt_tokens = usage.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    let completion_tokens = usage.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                    stream.usage = Some(json!({"prompt_tokens":prompt_tokens,"completion_tokens":completion_tokens,
                        "total_tokens":usage.get("total_tokens").and_then(Value::as_u64).unwrap_or(prompt_tokens.saturating_add(completion_tokens))}));
                }
                stream.done = true;
                stream.progress_events += 1;
            }
            _ => {}
        }
        Ok(())
    }
}
