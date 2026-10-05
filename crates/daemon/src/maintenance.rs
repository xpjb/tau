//! Offline ownership/restore operations. The same OS lease guards the serving
//! daemon and administrative writers; it is released on process death.
use anyhow::{Context,Result,ensure};
use std::path::Path;

pub(crate) struct DatabaseLease { _file:std::fs::File }
impl DatabaseLease {
    pub(crate) fn acquire(path:&Path)->Result<Self> {
        let parent=path.parent().context("Database has no parent")?;std::fs::create_dir_all(parent)?;
        let mut options=std::fs::OpenOptions::new();options.create(true).truncate(false).read(true).write(true);
        #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600).custom_flags(libc::O_NOFOLLOW);}
        let file=options.open(path.with_extension("writer.lock"))?;
        ensure!(file.metadata()?.is_file(),"Invalid database lease file");
        file.try_lock().context("Database is in use; stop its daemon before offline maintenance")?;
        Ok(Self {_file:file})
    }
}

/// Call only after restoring a consistent SQLite backup and its owned file
/// trees. This fences all pre-restore data references and never replays effects.
pub async fn rotate_lineage(path:&Path)->Result<String> {
    ensure!(path.is_absolute() && path.is_file(),"Specify an existing absolute database path");
    let _lease=DatabaseLease::acquire(path)?;
    let state=crate::state::StateStore::load(path.into()).await?;
    state.recover_blocks().await?;state.recover_operations().await?;
    state.access(|db| {
        let tx=db.transaction()?;let lineage=uuid::Uuid::new_v4().to_string();
        tx.execute("UPDATE block_state SET lineage=?1 WHERE singleton=1",[&lineage])?;
        tx.execute("INSERT INTO restore_guards SELECT id,?1 FROM sessions WHERE true ON CONFLICT(session_id) DO UPDATE SET lineage=excluded.lineage",[&lineage])?;
        // Pure inputs/descriptors from the previous generation are not ownership.
        tx.execute("DELETE FROM blocks WHERE scope IN ('@uploads','@control','@staging')",[])?;
        tx.execute("DELETE FROM block_changes WHERE scope IN ('@uploads','@control','@staging')",[])?;
        tx.commit()?;db.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")?;Ok(lineage)
    }).await
}

#[cfg(test)]
#[path = "../tests/unit/maintenance.rs"]
mod tests;
