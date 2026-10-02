//! Read-only audit of the actual bounded daemon index; prints sizes, not names.
//! cargo run -p tau-code-viewer --features filesystem --example index_size -- /dev/root
use anyhow::{Context, Result, ensure};
use std::{mem::size_of, path::PathBuf, time::{Duration, Instant}};
use tau_code_viewer::{filesystem::FileSystem, finder::PathIndex};
use tau_net::files::*;

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() -> Result<()> {
    let root = PathBuf::from(std::env::args_os().nth(1).context("Pass the directory to audit")?).canonicalize()?;
    ensure!(root.is_dir(), "Audit root must be a directory");
    let started = Instant::now();
    let filesystem = FileSystem::new(root.clone());
    let reply = loop {
        let reply = filesystem.request(root.clone(), FileRequest { session_id: "read-only-audit".into(), path: None,
            operation: FileOperation::Index { revision: None } }).await?;
        if matches!(&reply, FileReply::Index { indexing: false, .. }) { break reply; }
        ensure!(started.elapsed() < Duration::from_secs(25), "Index audit did not complete within 25s");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    let scan_ms = started.elapsed().as_millis();
    let index = PathIndex::apply(None, &reply)?;
    let FileReply::Index { limited, .. } = &reply else { unreachable!() };
    let json = serde_json::to_vec(&reply)?;
    // Match transfer/src/files.rs: independent 16 KiB chunks, zstd level 1,
    // falling back to raw bytes unless compression saves at least 16 bytes.
    let mut payload = 0;
    for chunk in json.chunks(16 * 1024) {
        let compressed = if chunk.len() >= 1024 { zstd::bulk::compress(chunk, 1)? } else { vec![] };
        payload += if !compressed.is_empty() && compressed.len() + 16 < chunk.len() { compressed.len() } else { chunk.len() };
    }
    let paths: usize = index.entries.iter().map(|p| p.path.len()).sum();
    let records = index.entries.len() * size_of::<IndexedPath>();
    let allocated_paths: usize = index.entries.iter().map(|p| p.path.capacity()).sum();
    let allocated_records = index.entries.capacity() * size_of::<IndexedPath>();
    let mut finder = tau_code_viewer::finder::Finder::new("main src");
    let matching = Instant::now();
    let matches = index.entries.iter().filter(|p| finder.score(&p.path).is_some()).count();
    println!("{}", serde_json::json!({
        "files": index.entries.len(), "visible_files": index.visible, "limited": limited,
        "scan_ms": scan_ms, "path_bytes": paths, "record_bytes": records,
        "snapshot_heap_bytes_minimum": allocated_paths + allocated_records,
        "json_bytes": json.len(), "chunk_compressed_payload_bytes": payload,
        "chunks": json.len().div_ceil(16 * 1024), "query": "main src", "matches": matches,
        "match_ms": matching.elapsed().as_millis(),
    }));
    Ok(())
}
