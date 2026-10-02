//! Short, consistent WAL snapshots independent of the serialized writer.
//! Connections are read-only; mutation/receipt transactions still use access().
use anyhow::Result;
use rusqlite::{Connection, OpenFlags};
use std::{path::Path, sync::{Arc, Mutex}, time::Duration};
use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const READERS: usize = 4;

pub(super) struct Readers {
    idle: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}
impl Readers {
    pub fn open(path: &Path) -> Result<Self> {
        let mut idle = Vec::with_capacity(READERS);
        for _ in 0..READERS {
            let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX)?;
            db.busy_timeout(Duration::from_secs(5))?;
            db.execute_batch("PRAGMA query_only=ON;")?;
            idle.push(db);
        }
        Ok(Self { idle: Mutex::new(idle), permits: Arc::new(Semaphore::new(READERS)) })
    }
    pub async fn read<T: Send + 'static>(self: &Arc<Self>, action: impl FnOnce(&Connection) -> Result<T> + Send + 'static) -> Result<T> {
        let mut timing=super::timing::Timing::new("reader");
        let permit = self.permits.clone().acquire_owned().await?;
        timing.acquired();
        let db = self.idle.lock().unwrap().pop().expect("reader permit owns a connection");
        let mut lease = Lease { db: Some(db), pool: self.clone(), _permit: permit };
        tokio::task::spawn_blocking(move || {
            // Every multi-query read sees one committed version. A writer can
            // commit alongside this snapshot; the next read sees that commit.
            timing.working();
            let result=(|| {
                let tx = lease.db.as_mut().unwrap().transaction()?;
                let value = action(&tx)?;
                tx.commit()?;
                Ok(value)
            })();
            timing.finished(result.is_ok());result
        }).await?
    }
}
struct Lease {
    db: Option<Connection>,
    pool: Arc<Readers>,
    _permit: OwnedSemaphorePermit,
}
impl Drop for Lease {
    fn drop(&mut self) {
        // Return on success, error, panic, or cancellation of the async caller.
        // The permit stays owned until the blocking read actually finishes.
        self.pool.idle.lock().unwrap().push(self.db.take().unwrap());
    }
}

#[cfg(test)]
#[path = "../tests/unit/state_reads.rs"]
mod tests;
