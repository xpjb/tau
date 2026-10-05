//! Pi's `proper-lockfile` contract: an empty `<auth path>.lock` directory,
//! refreshed mtime, and a 30-second stale lease. All refreshers reread the JSON
//! after acquiring it. No second service, sidecar credential, or runtime.
use anyhow::{Context, Result, bail, ensure};
use std::{fs::{File, Metadata}, path::{Path, PathBuf}, sync::{Arc, Mutex}, time::{Duration, Instant, SystemTime}};

const STALE: Duration = Duration::from_secs(30);
struct Owner { file: File, stamp: Metadata }
pub(super) struct AuthLock {
    path: PathBuf,
    owner: Arc<Mutex<Owner>>,
    heartbeat: tokio::task::JoinHandle<()>,
}
fn same(a: &Metadata, b: &Metadata) -> bool {
    #[cfg(unix)] {
        use std::os::unix::fs::MetadataExt;
        if a.dev() != b.dev() || a.ino() != b.ino() { return false; }
    }
    a.is_dir() && b.is_dir() && a.modified().ok() == b.modified().ok()
}
impl AuthLock {
    pub async fn acquire(file: &Path) -> Result<Self> {
        let mut path = file.as_os_str().to_owned(); path.push(".lock"); let path = PathBuf::from(path);
        let deadline = Instant::now() + STALE + Duration::from_secs(1);
        loop {
            let mut builder = std::fs::DirBuilder::new();
            #[cfg(unix)] { use std::os::unix::fs::DirBuilderExt; builder.mode(0o700); }
            match builder.create(&path) {
                Ok(()) => break,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                    if let Ok(stamp) = std::fs::symlink_metadata(&path) {
                        ensure!(stamp.is_dir(), "Credential lock is not a directory");
                        if stamp.modified()?.elapsed().is_ok_and(|age| age > STALE)
                            && std::fs::symlink_metadata(&path).is_ok_and(|now| same(&stamp, &now)) {
                            // Same stale empty directory. Never recursively remove
                            // contents or unlink a symlink/file masquerading as a lock.
                            match std::fs::remove_dir(&path) {
                                Ok(()) => continue,
                                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                                Err(error) => return Err(error).context("Cannot release stale credential lock"),
                            }
                        }
                    }
                    if Instant::now() >= deadline { bail!("Another app is updating the login. Try again shortly."); }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Err(error) => return Err(error).context("Cannot lock the credential file"),
            }
        }
        let open = (|| -> Result<Owner> {
            let mut options = std::fs::OpenOptions::new(); options.read(true);
            #[cfg(unix)] { use std::os::unix::fs::OpenOptionsExt; options.custom_flags(libc::O_NOFOLLOW | libc::O_DIRECTORY); }
            #[cfg(windows)] { use std::os::windows::fs::OpenOptionsExt; options.access_mode(0x100).custom_flags(0x02000000 | 0x00200000); }
            let file = options.open(&path)?; let stamp = file.metadata()?;
            ensure!(stamp.is_dir(), "Credential lock is not a directory");
            Ok(Owner { file, stamp })
        })();
        let owner = match open {
            Ok(owner) => Arc::new(Mutex::new(owner)),
            Err(error) => { let _ = std::fs::remove_dir(&path); return Err(error); }
        };
        let pulse = owner.clone(); let pulse_path = path.clone();
        let heartbeat = tokio::spawn(async move {
            loop {
                // Shorter than Pi's synchronous lock's ten-second stale default.
                tokio::time::sleep(Duration::from_secs(1)).await;
                let mut owner = pulse.lock().unwrap_or_else(|e| e.into_inner());
                if !std::fs::symlink_metadata(&pulse_path).is_ok_and(|now| same(&owner.stamp, &now)) { return; }
                if owner.file.set_modified(SystemTime::now()).is_err() { return; }
                let Ok(stamp) = owner.file.metadata() else { return; }; owner.stamp = stamp;
            }
        });
        Ok(Self { path, owner, heartbeat })
    }
    /// Fence token exchange/publication if a stale-lock takeover removed us.
    pub fn check(&self) -> Result<()> {
        let owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        ensure!(std::fs::symlink_metadata(&self.path).is_ok_and(|now| same(&owner.stamp, &now)), "Credential refresh lock was lost. Try again.");
        Ok(())
    }
}
impl Drop for AuthLock {
    fn drop(&mut self) {
        self.heartbeat.abort();
        let owner = self.owner.lock().unwrap_or_else(|e| e.into_inner());
        if std::fs::symlink_metadata(&self.path).is_ok_and(|now| same(&owner.stamp, &now)) { let _ = std::fs::remove_dir(&self.path); }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn shared_auth_lock_serializes_refreshers_and_recovers_stale_owners() {
        let dir=tempfile::tempdir().unwrap();let file=dir.path().join("auth.json");let path=dir.path().join("auth.json.lock");
        let first=AuthLock::acquire(&file).await.unwrap();
        assert!(tokio::time::timeout(Duration::from_millis(100),AuthLock::acquire(&file)).await.is_err());
        first.check().unwrap();drop(first);
        std::fs::create_dir(&path).unwrap();File::open(&path).unwrap().set_modified(SystemTime::now()-STALE-Duration::from_secs(1)).unwrap();
        let second=AuthLock::acquire(&file).await.unwrap();second.check().unwrap();drop(second);assert!(!path.exists());
    }
    #[tokio::test]
    async fn shared_auth_lock_fences_a_replaced_directory_without_removing_its_new_owner() {
        let dir=tempfile::tempdir().unwrap();let file=dir.path().join("auth.json");let path=dir.path().join("auth.json.lock");
        let first=AuthLock::acquire(&file).await.unwrap();std::fs::remove_dir(&path).unwrap();
        let second=AuthLock::acquire(&file).await.unwrap();assert!(first.check().is_err());drop(first);
        assert!(path.exists());second.check().unwrap();drop(second);assert!(!path.exists());
    }
    #[tokio::test]
    #[ignore = "Set TAU_PI_LOCK_MODULE to the deployed proper-lockfile index.js for real cross-runtime interoperability"]
    async fn shared_auth_lock_interoperates_with_deployed_pi_in_both_directions() {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        use std::process::Stdio;
        let module=std::env::var("TAU_PI_LOCK_MODULE").unwrap();let dir=tempfile::tempdir().unwrap();let file=dir.path().join("auth.json");
        std::fs::write(&file,"{}").unwrap();
        let mut node=tokio::process::Command::new("node").arg("-e").arg(r#"
            const lock=require(process.argv[1]);
            (async()=>{const release=await lock.lock(process.argv[2],{realpath:false,stale:30000});console.log('locked');
            process.stdin.once('data',async()=>{await release();console.log('released');process.exit(0)});})();
        "#).arg(&module).arg(&file).stdin(Stdio::piped()).stdout(Stdio::piped()).kill_on_drop(true).spawn().unwrap();
        let mut lines=tokio::io::BufReader::new(node.stdout.take().unwrap()).lines();assert_eq!(lines.next_line().await.unwrap().unwrap(),"locked");
        assert!(tokio::time::timeout(Duration::from_millis(150),AuthLock::acquire(&file)).await.is_err());
        node.stdin.take().unwrap().write_all(b"release\n").await.unwrap();assert!(node.wait().await.unwrap().success());
        let rust=AuthLock::acquire(&file).await.unwrap();
        // Keep the lease beyond Pi's synchronous ten-second stale threshold.
        tokio::time::sleep(Duration::from_secs(11)).await;
        let script=r#"const lock=require(process.argv[1]);try{const release=lock.lockSync(process.argv[2],{realpath:false});release();process.exit(2)}catch(e){process.exit(e.code==='ELOCKED'?0:3)}"#;
        assert!(tokio::process::Command::new("node").args(["-e",script,&module]).arg(&file).status().await.unwrap().success());
        rust.check().unwrap();drop(rust);
        assert!(tokio::process::Command::new("node").args(["-e","require(process.argv[1]).lockSync(process.argv[2],{realpath:false})()",&module]).arg(&file).status().await.unwrap().success());
    }
}
