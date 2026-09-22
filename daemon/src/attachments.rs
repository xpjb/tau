
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::path::Path;
use tokio_util::sync::CancellationToken;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

use crate::manager::{AgentManager, safe_file_name};
use crate::protocol::{UploadedFile, MAX_UPLOAD_BYTES};
use crate::transcript::{AttachmentKind, AttachmentRequest, attachment_request, FILE_LIMIT, IMAGE_LIMIT};

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
        if self.inner.state.get(id).await?.is_none() {
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

    pub async fn resolve_attachment(
        &self,
        id: &str,
        entry_id: &str,
    ) -> Result<ResolvedAttachment> {
        let entry = self.inner.state.entry(id,entry_id).await?;
        let request = attachment_request(&entry).context("Entry has no Tau attachment")?;
        open_attachment(&self.inner.config.attachment_root, &request).await
    }
}

pub(crate) fn image_mime(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") { Some("image/png") }
    else if bytes.starts_with(&[0xff,0xd8,0xff]) { Some("image/jpeg") }
    else if bytes.len() >= 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" { Some("image/webp") }
    else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") { Some("image/gif") }
    else if bytes.starts_with(b"BM") { Some("image/bmp") } else { None }
}
pub(crate) async fn regular_file(path: &Path) -> Result<fs::File> {
    let mut options = fs::OpenOptions::new(); options.read(true);
    #[cfg(unix)] options.custom_flags(libc::O_NONBLOCK);
    let file = options.open(path).await?;
    if !file.metadata().await?.is_file() { bail!("Path is not a regular file"); }
    Ok(file)
}
async fn open_attachment(root: &Path, request: &AttachmentRequest) -> Result<ResolvedAttachment> {
    let root = fs::canonicalize(root).await.context("Tau attachment root is unavailable")?;
    let path = fs::canonicalize(&request.path).await.context("Attachment is unavailable")?;
    if !path.starts_with(&root) || path == root { bail!("Attachment is outside the Tau outbox"); }
    let mut file = regular_file(&path).await?;
    let size = file.metadata().await?.len();
    let limit = match request.kind { AttachmentKind::Image => IMAGE_LIMIT, AttachmentKind::File => FILE_LIMIT };
    if size > limit { bail!("Attachment exceeds the {limit} byte limit"); }
    let mime_type = if request.kind == AttachmentKind::Image {
        let mut header = [0; 12]; let length = file.read(&mut header).await?;
        file.seek(std::io::SeekFrom::Start(0)).await?;
        match image_mime(&header[..length]) {
            Some(mime @ ("image/png" | "image/jpeg" | "image/webp")) => mime,
            _ => bail!("Attachment is not a supported image"),
        }
    } else { "application/octet-stream" };
    let file_name = path.file_name().context("Attachment has no file name")?.to_string_lossy().into_owned();
    Ok(ResolvedAttachment { file, file_name, mime_type, size })
}

async fn attachment_result(path: &Path, file: &mut fs::File, caption: Option<&str>) -> Result<Value> {
    let size = file.metadata().await?.len();
    if size > FILE_LIMIT { bail!("Attachment exceeds the {FILE_LIMIT} byte limit"); }
    file.seek(std::io::SeekFrom::Start(0)).await?;
    let mut header = [0; 12]; let length = file.read(&mut header).await?;
    let image = size <= IMAGE_LIMIT && matches!(image_mime(&header[..length]), Some("image/png" | "image/jpeg" | "image/webp"));
    Ok(json!({"content":[{"type":"text","text":format!("{} queued for Tau: {}", if image {"Image"} else {"File"}, path.file_name().unwrap().to_string_lossy())}],
        "details":{"tauAttachment":{"version":1,"kind":if image {"image"} else {"file"},"path":path,"caption":caption,"size":size}}}))
}

// Both local files and provider-generated bytes take this path. A partial/cancelled
// copy is removed; a published attachment has its own private, durable file.
async fn stage_attachment(root: &Path, name: &std::ffi::OsStr, source: impl tokio::io::AsyncRead + Unpin, caption: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
    let directory = tempfile::Builder::new().prefix(".tau-").tempdir_in(root)?;
    let path = directory.path().join(name);
    let mut options = fs::OpenOptions::new(); options.create_new(true).read(true).write(true);
    #[cfg(unix)] options.mode(0o600);
    let mut file = options.open(&path).await?;
    let mut source = source.take(FILE_LIMIT + 1);
    tokio::select! {
        _ = cancel.cancelled() => bail!("Attachment staging cancelled"),
        result = tokio::io::copy(&mut source, &mut file) => { result?; }
    }
    file.flush().await?;
    let result = attachment_result(&path, &mut file, caption).await?;
    file.sync_all().await?;
    fs::File::open(directory.path()).await?.sync_all().await?;
    fs::File::open(root).await?.sync_all().await?;
    if cancel.is_cancelled() { bail!("Attachment staging cancelled"); }
    let _ = directory.keep();
    Ok(result)
}

pub(crate) async fn send_file(root: &Path, source: &Path, caption: Option<&str>, cancel: &CancellationToken) -> Result<Value> {
    let caption = caption.map(str::trim).filter(|value| !value.is_empty());
    if caption.is_some_and(|value| value.chars().count() > 1024) { bail!("Caption exceeds 1024 characters"); }
    if cancel.is_cancelled() { bail!("Attachment staging cancelled"); }
    let source = fs::canonicalize(source).await.context("Attachment does not exist")?;
    let mut file = regular_file(&source).await?;
    if file.metadata().await?.len() > FILE_LIMIT { bail!("Attachment exceeds the {FILE_LIMIT} byte limit"); }
    let root = fs::canonicalize(root).await.context("Tau attachment root is unavailable")?;
    if source.starts_with(&root) { attachment_result(&source, &mut file, caption).await }
    else { stage_attachment(&root, source.file_name().context("Attachment has no file name")?, file, caption, cancel).await }
}

pub(crate) async fn generated_image(root: &Path, bytes: &[u8], cancel: &CancellationToken) -> Result<Value> {
    let root = fs::canonicalize(root).await.context("Tau attachment root is unavailable")?;
    let name = format!("generated-{}.png", uuid::Uuid::new_v4());
    stage_attachment(&root, std::ffi::OsStr::new(&name), bytes, None, cancel).await
}

// store:false cannot replay a generated-image item by ID. Rehydrate the staged
// image only when constructing provider context, never into history or the wire.
pub(crate) async fn image_reference(root: &Path, request: &AttachmentRequest) -> Result<Value> {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    let resolved = open_attachment(root, request).await?;
    let mut bytes = Vec::new(); resolved.file.take(IMAGE_LIMIT + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > IMAGE_LIMIT { bail!("Reference image exceeds 10 MB"); }
    Ok(json!({"type":"image_url","image_url":{"url":format!("data:{};base64,{}", resolved.mime_type, STANDARD.encode(bytes))}}))
}
