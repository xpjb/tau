use std::path::{Path, PathBuf};
use std::process::Stdio;
use anyhow::{Context, Result, bail};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncSeekExt};
use tokio_util::sync::CancellationToken;
use crate::config::Config;
use crate::settings::Settings;
use crate::state::StateStore;
use crate::transcript::{FILE_LIMIT, IMAGE_LIMIT};

pub fn definitions(config: &Config) -> Vec<Value> {
    [
        ("read", "Read text or an image. Text is bounded to 2000 lines / 50 KB; use offset/limit to continue.", json!({"path":{"type":"string"},"offset":{"type":"integer","minimum":1},"limit":{"type":"integer","minimum":1}}), vec!["path"]),
        ("bash", "Execute bash in the working directory. Output is bounded; full output is saved to a file. Timeout is optional, in seconds.", json!({"command":{"type":"string"},"timeout":{"type":"integer","minimum":1}}), vec!["command"]),
        ("write", "Write a file, creating parent directories. Replaces existing contents.", json!({"path":{"type":"string"},"content":{"type":"string"}}), vec!["path","content"]),
        ("edit", "Apply exact replacements against the original file. Each oldText must be unique; edits must not overlap.", json!({"path":{"type":"string"},"edits":{"type":"array","items":{"type":"object","properties":{"oldText":{"type":"string"},"newText":{"type":"string"}},"required":["oldText","newText"],"additionalProperties":false}}}), vec!["path","edits"]),
        ("send_image", "Send a PNG, JPEG or WebP file from the Tau outbox to the user.", json!({"path":{"type":"string"},"caption":{"type":"string"}}), vec!["path"]),
        ("send_file", "Send a file from the Tau outbox to the user.", json!({"path":{"type":"string"},"caption":{"type":"string"}}), vec!["path"]),
        ("flag_it", "Log an incidental finding outside the task. State location and impact, omit secrets, and continue the current task.", json!({"str":{"type":"string","minLength":1,"maxLength":4096}}), vec!["str"]),
        ("web_search", "Search the web.", json!({"query":{"type":"string"}}), vec!["query"]),
    ].into_iter().map(|(name, description, properties, required)| json!({"name":name,
        "description":if name.starts_with("send_") { format!("{description} Stage the file under {} first.", config.attachment_root.display()) } else { description.to_owned() },
        "parameters":{"type":"object","properties":properties,"required":required,"additionalProperties":false}})).collect()
}

pub fn text_result(text: impl Into<String>) -> Value { json!({"content":[{"type":"text","text":text.into()}]}) }
pub fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
    else if bytes.starts_with(&[0xff,0xd8,0xff]) { Some("image/jpeg") }
    else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { Some("image/webp") }
    else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") { Some("image/gif") }
    else if bytes.starts_with(b"BM") { Some("image/bmp") } else { None }
}
fn path(config: &Config, input: &str) -> PathBuf {
    let input = input.strip_prefix('@').unwrap_or(input);
    if let Some(rest) = input.strip_prefix("~/") { PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest) }
    else { config.cwd.join(input) }
}
fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str> { value[field].as_str().with_context(|| format!("{field} must be a string")) }

pub async fn execute(config: &Config, settings: &Settings, state: &StateStore, session: &str, name: &str, args: &Value, cancel: &CancellationToken) -> Result<Value> {
    if cancel.is_cancelled() { bail!("Tool cancelled"); }
    match name {
        "read" => {
            let path = path(config, string(args, "path")?);
            let mut file = tokio::fs::File::open(&path).await?;
            if !file.metadata().await?.is_file() { bail!("Path is not a regular file"); }
            let mut header = [0; 12]; let length = file.read(&mut header).await?;
            file.seek(std::io::SeekFrom::Start(0)).await?;
            if let Some(mime) = image_mime(&header[..length]) {
                let mut bytes = Vec::new(); file.take(IMAGE_LIMIT + 1).read_to_end(&mut bytes).await?;
                if bytes.len() as u64 > IMAGE_LIMIT { bail!("Image exceeds 10 MB"); }
                return Ok(json!({"content":[{"type":"image", "mimeType":mime, "data":STANDARD.encode(bytes)}]}));
            }
            use tokio::io::AsyncBufReadExt;
            let offset = args.get("offset").map(|v| v.as_u64().filter(|v| *v > 0).context("offset must be positive")).transpose()?.unwrap_or(1);
            let limit = args.get("limit").map(|v| v.as_u64().filter(|v| *v > 0).context("limit must be positive")).transpose()?.unwrap_or(2000).min(2000);
            let mut reader = tokio::io::BufReader::new(file); let mut line = Vec::new(); let mut output = String::new();
            let mut index = 0; let mut truncated = false;
            loop {
                line.clear();
                // A single giant line must not allocate an unbounded buffer.
                let count = (&mut reader).take(50 * 1024 + 1).read_until(b'\n', &mut line).await?;
                if count == 0 { break; }
                index += 1;
                if count > 50 * 1024 { bail!("Line {index} exceeds 50 KB; use bash for a bounded byte range"); }
                if index < offset { continue; }
                if index >= offset + limit || output.len() + count > 50 * 1024 { truncated = true; break; }
                output.push_str(&String::from_utf8(line.clone()).context("File is not UTF-8 text or a supported image")?);
            }
            if truncated { output.push_str(&format!("\n[Truncated; continue with offset {index}]") ); }
            Ok(text_result(output))
        }
        "write" | "edit" => {
            let input = path(config, string(args, "path")?);
            let path = match tokio::fs::canonicalize(&input).await { Ok(path) => path, Err(e) if e.kind() == std::io::ErrorKind::NotFound && name == "write" => input, Err(e) => return Err(e.into()) };
            let text = if name == "write" { string(args, "content")?.to_owned() } else {
                let original = tokio::fs::read_to_string(&path).await?;
                let edits = args["edits"].as_array().filter(|v| !v.is_empty()).context("edits must be a nonempty array")?;
                let mut ranges = Vec::new();
                for edit in edits {
                    let old = string(edit, "oldText")?; let new = string(edit, "newText")?;
                    if old.is_empty() { bail!("oldText cannot be empty"); }
                    let mut matches = original.match_indices(old);
                    let (start, _) = matches.next().context("oldText was not found")?;
                    if matches.next().is_some() { bail!("oldText must match exactly once"); }
                    ranges.push((start, start + old.len(), new));
                }
                ranges.sort_by_key(|range| range.0);
                if ranges.windows(2).any(|w| w[0].1 > w[1].0) { bail!("Edits overlap; combine them"); }
                let mut text = original;
                for (start, end, new) in ranges.into_iter().rev() { text.replace_range(start..end, new); }
                text
            };
            let permissions = tokio::fs::metadata(&path).await.ok().map(|m| m.permissions());
            crate::settings::atomic_write(&path, text.as_bytes()).await?;
            let file = tokio::fs::OpenOptions::new().write(true).open(&path).await?;
            if let Some(permissions) = permissions { file.set_permissions(permissions).await?; }
            file.sync_all().await?;
            Ok(text_result(format!("{} {}", if name == "write" { "Wrote" } else { "Edited" }, path.display())))
        }
        "bash" => {
            let command = string(args, "command")?;
            let seconds = args.get("timeout").map(|value| value.as_u64().filter(|v| *v > 0).context("timeout must be positive seconds")).transpose()?;
            let directory = config.state_path.parent().unwrap_or(Path::new(".")).join("tool-output");
            tokio::fs::create_dir_all(&directory).await?;
            let output_path = directory.join(format!("{}.log", uuid::Uuid::new_v4()));
            let output = std::fs::OpenOptions::new().write(true).create_new(true).open(&output_path)?;
            let mut process = tokio::process::Command::new(&settings.agent.shell_path);
            process.arg("-c").arg(format!("{}\n{command}", settings.agent.shell_command_prefix)).current_dir(&config.cwd)
                .stdin(Stdio::null()).stdout(output.try_clone()?).stderr(output).kill_on_drop(true)
                .env_remove("TAU_TOKEN").env_remove("TAU_FLAG_TOKEN");
            #[cfg(unix)] process.process_group(0);
            let mut child = process.spawn().context("Could not start bash")?;
            struct Group(u32);
            impl Drop for Group {
                fn drop(&mut self) { #[cfg(unix)] unsafe { libc::kill(-(self.0 as i32), libc::SIGKILL); } }
            }
            let group = Group(child.id().context("Shell has no process ID")?);
            let timeout = async { match seconds { Some(seconds) => tokio::time::sleep(std::time::Duration::from_secs(seconds)).await, None => std::future::pending::<()>().await } };
            let status = tokio::select! {
                result = child.wait() => result.map(|status| format!("Exit status: {status}"))?,
                _ = cancel.cancelled() => "Cancelled".into(),
                _ = timeout => "Timed out".into(),
            };
            drop(group);
            if child.try_wait()?.is_none() { let _ = child.kill().await; let _ = child.wait().await; }
            let mut file = tokio::fs::File::open(&output_path).await?;
            let length = file.metadata().await?.len();
            let start = length.saturating_sub(settings.agent.max_tool_output_bytes as u64);
            file.seek(std::io::SeekFrom::Start(start)).await?;
            let mut bytes = Vec::new(); file.take(settings.agent.max_tool_output_bytes as u64).read_to_end(&mut bytes).await?;
            let text = String::from_utf8_lossy(&bytes);
            let lines = text.lines().collect::<Vec<_>>();
            let visible = lines[lines.len().saturating_sub(2000)..].join("\n");
            let truncated = start > 0 || lines.len() > 2000;
            if !truncated { tokio::fs::remove_file(&output_path).await?; }
            Ok(text_result(format!("{visible}\n\n{status}{}", if truncated { format!("\nOutput truncated. Full output: {}", output_path.display()) } else { String::new() })))
        }
        "send_file" | "send_image" => {
            let path = tokio::fs::canonicalize(path(config, string(args, "path")?)).await?;
            let root = tokio::fs::canonicalize(&config.attachment_root).await?;
            if !path.starts_with(&root) || path == root { bail!("Attachment is outside the Tau outbox"); }
            let mut file = tokio::fs::File::open(&path).await?; let metadata = file.metadata().await?;
            if !metadata.is_file() { bail!("Attachment must be a regular file"); }
            let image = name == "send_image";
            if metadata.len() > if image { IMAGE_LIMIT } else { FILE_LIMIT } { bail!("Attachment exceeds size limit"); }
            if image {
                let mut header = [0;12]; let length = file.read(&mut header).await?;
                if !matches!(image_mime(&header[..length]), Some("image/png" | "image/jpeg" | "image/webp")) { bail!("send_image accepts PNG, JPEG or WebP"); }
            }
            let caption = args.get("caption").map(|_| string(args, "caption")).transpose()?.map(str::trim).filter(|v| !v.is_empty());
            if caption.is_some_and(|v| v.chars().count() > 1024) { bail!("Caption exceeds 1024 characters"); }
            let mut result = text_result(format!("Queued for Tau: {}", path.file_name().unwrap().to_string_lossy()));
            result["details"] = json!({"tauAttachment":{"version":1,"kind":if image {"image"} else {"file"},"path":path,"caption":caption,"size":metadata.len()}});
            Ok(result)
        }
        "flag_it" => {
            let saved = state.flag(session, string(args, "str")?).await?;
            let mut result = text_result(format!("Flag {} saved. Continue the current task.", saved.id));
            result["details"] = json!({"flagId":saved.id}); Ok(result)
        }
        _ => bail!("Unknown tool {name}"),
    }
}
