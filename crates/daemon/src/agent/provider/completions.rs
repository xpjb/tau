use anyhow::{bail, Context as _, Result};
use serde_json::{json, Value};
use super::{Decoder, Stream};
pub struct Completions;
impl Decoder for Completions {
    fn decode(&self, event: &Value, stream: &mut Stream<'_>) -> Result<()> {
        if event == "[DONE]" {
            stream.done = true;
            return Ok(());
        }
        if let Some(id) = event.get("id").and_then(Value::as_str) {
            stream.id = Some(id.to_string());
        }
        if let Some(provider) = event.get("provider").and_then(Value::as_str) {
            stream.provider = Some(provider.to_string());
        }
        if let Some(model) = event.get("model").and_then(Value::as_str) {
            stream.model = Some(model.to_string());
        }
        if let Some(error) = event.get("error").filter(|error| !error.is_null()) {
            stream.error = Some(error.clone());
        }
        if let Some(usage) = event.get("usage").filter(|usage| !usage.is_null()) {
            stream.usage = Some(
                serde_json::from_value(usage.clone())
                    .context("openrouter returned invalid streaming usage")?,
            );
        }

        let Some(choice) = event
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|choices| {
                choices
                    .iter()
                    .find(|choice| choice.get("index").and_then(Value::as_u64) == Some(0))
                    .or_else(|| choices.first())
            })
        else {
            return Ok(());
        };
        if let Some(finish_reason) = choice.get("finish_reason").and_then(Value::as_str) {
            stream.finish_reason = Some(finish_reason.to_string());
        }
        let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
            return Ok(());
        };
        let mut made_progress = false;
        for (key, fragment) in delta {
            match key.as_str() {
                "tool_calls" => {
                    let fragments = fragment
                        .as_array()
                        .context("streaming tool_calls was not an array")?;
                    for fragment in fragments {
                        let index = fragment
                            .get("index")
                            .and_then(Value::as_u64)
                            .context("streaming tool call had no index")?
                            as usize;
                        let fragment = fragment
                            .as_object()
                            .context("streaming tool call was not an object")?;
                        let tool_call = stream.tool_calls.entry(index).or_default();
                        for (key, value) in fragment {
                            if key == "index" || value.is_null() {
                                continue;
                            }
                            if key != "function" {
                                tool_call.insert(key.clone(), value.clone());
                                continue;
                            }
                            let fields = value
                                .as_object()
                                .context("streaming tool function was not an object")?;
                            let function = tool_call
                                .entry("function")
                                .or_insert_with(|| json!({}))
                                .as_object_mut()
                                .context("assembled tool function was not an object")?;
                            for (key, value) in fields {
                                if value.is_null() {
                                    continue;
                                }
                                if matches!(key.as_str(), "name" | "arguments") {
                                    let value = value.as_str().with_context(|| {
                                        format!("streaming tool function {key} was not text")
                                    })?;
                                    if !value.is_empty() {
                                        made_progress = true;
                                    }
                                    let Value::String(assembled) = function
                                        .entry(key)
                                        .or_insert_with(|| Value::String(String::new()))
                                    else {
                                        bail!("assembled tool function field was not text");
                                    };
                                    assembled.push_str(value);
                                } else {
                                    function.insert(key.clone(), value.clone());
                                }
                            }
                        }
                    }
                }
                "content" | "reasoning" | "refusal" => {
                    if fragment.is_null() {
                        continue;
                    }
                    let fragment = fragment
                        .as_str()
                        .with_context(|| format!("streaming assistant {key} was not text"))?;
                    if !fragment.is_empty() {
                        made_progress = true;
                    }
                    let Value::String(assembled) = stream
                        .message
                        .entry(key)
                        .or_insert_with(|| Value::String(String::new()))
                    else {
                        bail!("assembled assistant field was not text");
                    };
                    assembled.push_str(fragment);
                }
                "reasoning_details" | "annotations" => {
                    if fragment.is_null() {
                        continue;
                    }
                    let fragment = fragment.as_array().with_context(|| {
                        format!("streaming assistant {key} was not an array")
                    })?;
                    if !fragment.is_empty() {
                        made_progress = true;
                    }
                    stream.message
                        .entry(key)
                        .or_insert_with(|| Value::Array(Vec::new()))
                        .as_array_mut()
                        .context("assembled assistant field was not an array")?
                        .extend(fragment.iter().cloned());
                }
                "role" => {
                    if !fragment.is_null() {
                        stream.message.insert(key.clone(), fragment.clone());
                    }
                }
                _ => {
                    if !fragment.is_null() {
                        stream.message.insert(key.clone(), fragment.clone());
                    }
                }
            }
        }
        if made_progress {
            stream.progress_events += 1;
        }
        Ok(())
    }
}
