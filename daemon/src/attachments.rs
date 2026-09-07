use std::collections::HashMap;

use anyhow::{Context, Result, bail};
use serde_json::Value;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::manager::{AgentManager, safe_file_name};
use crate::protocol::{UploadedFile, MAX_UPLOAD_BYTES};
use crate::transcript::{AttachmentKind, Entry, attachment_request, FILE_LIMIT, IMAGE_LIMIT};

pub struct ResolvedAttachment {
    pub file: fs::File,
    pub file_name: String,
    pub mime_type: &'static str,
    pub size: u64,
}

impl AgentManager {
    pub async fn store_upload(
        &self,
        id: &str,
        file_name: &str,
        bytes: &[u8],
    ) -> Result<UploadedFile> {
        if bytes.is_empty() {
            bail!("attached file is empty");
        }
        if bytes.len() > MAX_UPLOAD_BYTES {
            bail!("attached file exceeds Tau's upload limit");
        }
        let runtime = self.runtime(id).await?;
        let _guard = runtime.operation.lock().await;
        if self.inner.state.get(id).is_none() {
            bail!("unknown session {id}");
        }

        let safe_name = safe_file_name(file_name);

        fs::create_dir_all(&self.inner.config.upload_root).await?;
        let root = fs::canonicalize(&self.inner.config.upload_root).await?;
        let directory = root.join(id);
        fs::create_dir_all(&directory).await?;
        let directory = fs::canonicalize(directory).await?;
        if !directory.starts_with(&root) || directory == root {
            bail!("unsafe Tau upload directory");
        }
        let path = directory.join(format!("{}-{safe_name}", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&path)
            .await?;
        file.write_all(bytes).await?;
        file.flush().await?;
        file.sync_all().await?;
        Ok(UploadedFile {
            name: safe_name,
            path: path.to_string_lossy().into_owned(),
            size: bytes.len().try_into().unwrap_or(u64::MAX),
        })
    }

    pub(crate) async fn populate_attachment_sizes(&self, entries: &[Value], messages: &mut [Entry]) {
        let Ok(root) = fs::canonicalize(&self.inner.config.attachment_root).await else {
            return;
        };
        let by_id = entries
            .iter()
            .filter_map(|entry| Some((entry.get("id")?.as_str()?, entry)))
            .collect::<HashMap<_, _>>();
        for message in messages {
            let Some(attachment) = message.attachment.as_mut() else {
                continue;
            };
            if attachment.size.is_some() {
                continue;
            }
            let Some(request) = by_id.get(message.id.as_str()).and_then(|entry| {
                attachment_request(entry)
            }) else {
                continue;
            };
            let Ok(path) = fs::canonicalize(&request.path).await else {
                continue;
            };
            if !path.starts_with(&root) {
                continue;
            }
            let Ok(metadata) = fs::metadata(path).await else {
                continue;
            };
            let limit = match request.kind {
                AttachmentKind::Image => IMAGE_LIMIT,
                AttachmentKind::File => FILE_LIMIT,
            };
            if metadata.is_file() && metadata.len() <= limit {
                attachment.size = Some(metadata.len());
                message.measure_saved_bytes();
            }
        }
    }

    pub async fn resolve_attachment(
        &self,
        id: &str,
        entry_id: &str,
    ) -> Result<ResolvedAttachment> {
        let runtime = self.runtime(id).await?;
        let cached = {
            let content = runtime.content.lock().await;
            if let Some(transcript) = &content.transcript {
                let attachment = transcript.entry(entry_id).and_then(|entry| entry.attachment.as_ref())
                    .context("entry has no Tau attachment")?;
                Some(crate::transcript::AttachmentRequest {
                    kind: attachment.kind,
                    path: attachment.source_path.clone().context("attachment source is unavailable")?,
                    caption: attachment.caption.clone(),
                    size: attachment.size,
                })
            } else { None }
        };
        let request = if let Some(request) = cached { request } else {
            let process = runtime.content.lock().await.process.clone();
            let (entries, _) = self.entries_for_read(id, process.as_ref()).await?;
            let entry = entries.iter().find(|entry| entry.get("id").and_then(Value::as_str) == Some(entry_id))
                .with_context(|| format!("attachment entry {entry_id} does not exist"))?;
            attachment_request(entry).context("entry has no Tau attachment")?
        };
        let root = fs::canonicalize(&self.inner.config.attachment_root)
            .await
            .context("Tau attachment root is unavailable")?;
        let path = fs::canonicalize(&request.path)
            .await
            .with_context(|| format!("attachment {} is unavailable", request.path.display()))?;
        if !path.starts_with(&root) {
            bail!("attachment is outside the Tau outbox");
        }
        let mut file = fs::File::open(&path).await?;
        let metadata = file.metadata().await?;
        if !metadata.is_file() {
            bail!("attachment is not a regular file");
        }
        let limit = match request.kind {
            AttachmentKind::Image => IMAGE_LIMIT,
            AttachmentKind::File => FILE_LIMIT,
        };
        if metadata.len() > limit {
            bail!("attachment exceeds the {} byte limit", limit);
        }
        let mime_type = match request.kind {
            AttachmentKind::File => "application/octet-stream",
            AttachmentKind::Image => {
                let mut header = [0_u8; 12];
                let length = file.read(&mut header).await?;
                file.seek(std::io::SeekFrom::Start(0)).await?;
                if length >= 8 && header[..8] == [137, 80, 78, 71, 13, 10, 26, 10] {
                    "image/png"
                } else if length >= 3 && header[..3] == [0xff, 0xd8, 0xff] {
                    "image/jpeg"
                } else if length >= 12
                    && &header[..4] == b"RIFF"
                    && &header[8..12] == b"WEBP"
                {
                    "image/webp"
                } else {
                    bail!("attachment is not a supported image");
                }
            }
        };
        let file_name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .filter(|name| !name.is_empty())
            .context("attachment has no file name")?;
        Ok(ResolvedAttachment {
            file,
            file_name,
            mime_type,
            size: metadata.len(),
        })
    }
}
