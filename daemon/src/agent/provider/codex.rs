use anyhow::{bail, Context as _, Result};
use serde_json::{json, Value};
use super::{Decoder, Stream, GeneratedImage};
use base64::{Engine as _, engine::general_purpose::STANDARD};
const MAX_GENERATED_BYTES: usize = crate::transcript::IMAGE_LIMIT as usize;
const MAX_GENERATED_FILES: usize = 4;
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
                let segment = (kind == "response.reasoning_summary_text.delta",
                    event.get("output_index").and_then(Value::as_u64),
                    event.get("summary_index").and_then(Value::as_u64));
                let boundary = key == "reasoning" && !text.is_empty()
                    && stream.reasoning_segment.is_some_and(|previous| previous != segment);
                let Value::String(assembled) = stream.message.entry(key)
                    .or_insert_with(|| Value::String(String::new()))
                else {
                    bail!("assembled Codex text had the wrong type");
                };
                if boundary && !assembled.is_empty() {
                    // A lone LF is a Markdown soft break, so adjacent summary
                    // headings would still display on the same line. Separate
                    // parts (not SSE chunks) as paragraphs, preserving any LF
                    // already supplied by the model on either side.
                    let breaks = assembled.bytes().rev().take_while(|&b| b == b'\n').count()
                        + text.bytes().take_while(|&b| b == b'\n').count();
                    for _ in breaks..2 { assembled.push('\n'); }
                }
                assembled.push_str(text);
                if !text.is_empty() {
                    if key == "reasoning" { stream.reasoning_segment = Some(segment); }
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
                let mut file_ids = HashSet::new();
                let mut files = Vec::new();
                let mut file_bytes = 0;
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
                        Some("image_generation_call") => {
                            if item["status"] != "completed" { bail!("Codex image generation did not complete"); }
                            let id = item["id"].as_str().filter(|id| !id.is_empty()).context("Codex image has no ID")?.to_owned();
                            if !file_ids.insert(id) { bail!("Codex returned a duplicate generated image ID"); }
                            if !matches!(item.get("output_format").and_then(Value::as_str), None | Some("png")) {
                                bail!("Codex returned an unsupported image format");
                            }
                            let encoded = item["result"].take();
                            let encoded = encoded.as_str().context("Codex image has no encoded data")?;
                            if files.len() >= MAX_GENERATED_FILES || encoded.len() > (MAX_GENERATED_BYTES - file_bytes).div_ceil(3) * 4 {
                                bail!("Codex generated images exceeded the size or count limit");
                            }
                            let bytes = STANDARD.decode(encoded).context("Codex image has invalid base64 data")?;
                            file_bytes += bytes.len();
                            if file_bytes > MAX_GENERATED_BYTES || !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                                bail!("Codex image exceeded its size limit or was not PNG");
                            }
                            files.push(GeneratedImage { bytes });
                            // Bytes and non-replayable image IDs must not enter model history,
                            // live transcript snapshots, logs or the client protocol.
                            *item = Value::Null;
                        }
                        Some("reasoning" | "compaction") => {}
                        _ => bail!("unsupported Codex output item"),
                    }
                }
                output.retain(|item| !item.is_null());
                stream.images = files;
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

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reasoning_summary_parts_are_lines_but_transport_chunks_are_not() {
        let mut stream = Stream::new(&Codex);
        for (index, text) in [(0, "Check"), (0, " the data"), (1, "Compare"), (1, " results")] {
            Codex.decode(&json!({"type":"response.reasoning_summary_text.delta",
                "output_index":0,"summary_index":index,"delta":text}), &mut stream).unwrap();
        }
        assert_eq!(stream.assistant_message()["reasoning"], "Check the data\n\nCompare results");
        Codex.decode(&json!({"type":"response.reasoning_summary_text.delta",
            "output_index":1,"summary_index":0,"delta":"Final check"}), &mut stream).unwrap();
        assert_eq!(stream.assistant_message()["reasoning"], "Check the data\n\nCompare results\n\nFinal check");
        Codex.decode(&json!({"type":"response.reasoning_summary_text.delta",
            "output_index":1,"summary_index":1,"delta":"\nAlready separated"}), &mut stream).unwrap();
        assert_eq!(stream.assistant_message()["reasoning"], "Check the data\n\nCompare results\n\nFinal check\n\nAlready separated");
    }
    #[test]
    fn generated_images_enforce_encoding_format_individual_cumulative_and_count_bounds() {
        let image = |id: &str, result: String| json!({"type":"image_generation_call","id":id,"status":"completed","result":result});
        let png = STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        let mut half = b"\x89PNG\r\n\x1a\n".to_vec(); half.resize(MAX_GENERATED_BYTES / 2 + 1, 0);
        let invalid = [
            vec![image("big", "A".repeat(MAX_GENERATED_BYTES.div_ceil(3)*4+4))],
            vec![image("half1", STANDARD.encode(&half)),image("half2", STANDARD.encode(&half))],
            (0..=MAX_GENERATED_FILES).map(|i| image(&i.to_string(),png.clone())).collect(),
            vec![image("not-png", STANDARD.encode(b"not a PNG"))],
        ];
        for output in invalid {
            let mut stream = Stream::new(&Codex);
            assert!(Codex.decode(&json!({"type":"response.completed","response":{"status":"completed","output":output}}),&mut stream).is_err());
            assert!(stream.images.is_empty(), "Validation failure must not expose even the valid images in the same response");
        }
        let mut stream = Stream::new(&Codex);
        Codex.decode(&json!({"type":"response.completed","response":{"status":"completed","output":(0..MAX_GENERATED_FILES).map(|i| image(&i.to_string(),png.clone())).collect::<Vec<_>>()}}),&mut stream).unwrap();
        assert!(stream.done); assert_eq!(stream.images.len(),MAX_GENERATED_FILES);
        assert!(!stream.assistant_message().to_string().contains(&png));
    }
}
