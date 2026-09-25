//! Private client file bounds. Only disposable hashed downloads are evicted;
//! authored imports and arbitrary user export destinations are never collected.
use anyhow::{Result,ensure};
use std::{path::{Path,PathBuf},time::{Duration,SystemTime}};
pub(crate) fn files(root:&Path)->Result<Vec<(PathBuf,u64,SystemTime)>> {
    if !root.exists() {return Ok(vec![]);}
    let mut pending=vec![(root.to_owned(),0)];let mut files=vec![];let mut entries=0;
    while let Some((path,depth))=pending.pop() {
        ensure!(depth<=8,"Private storage tree is too deep; inspect it before adding files");
        for entry in std::fs::read_dir(path)? {
            let entry=entry?;entries+=1;ensure!(entries<=100000,"Private storage entry quota reached");
            let kind=entry.file_type()?;
            if kind.is_dir() {pending.push((entry.path(),depth+1));}
            else if kind.is_file() {let meta=entry.metadata()?;files.push((entry.path(),meta.len(),meta.modified()?));}
        }
    }Ok(files)
}
pub(crate) fn admit_import(root:&Path,size:u64)->Result<()> {
    let files=files(root)?;
    ensure!(files.len()<8192 && files.iter().map(|(_,size,_)|size).sum::<u64>().saturating_add(size)<=2*1024*1024*1024,
        "Saved attachment storage is full (2 GiB / 8192 files). Remove authored attachments explicitly before importing more");Ok(())
}
pub(crate) fn collect_downloads(root:&Path,target:&Path,size:u64,limit:u64)->Result<()> {
    let mut files=files(root)?.into_iter().filter(|(path,_,_)|path.strip_prefix(root).is_ok_and(|relative|
        relative.components().count()==3 && relative.components().all(|c|c.as_os_str().to_str().is_some_and(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit()))))).collect::<Vec<_>>();
    files.sort_by_key(|(_,_,time)|*time);
    let mut bytes=files.iter().filter(|(path,_,_)|path!=target).map(|(_,size,_)|size).sum::<u64>().saturating_add(size);
    let mut count=files.len()+1;
    for (path,size,time) in files {
        if path==target {continue;}
        let expired=SystemTime::now().duration_since(time).unwrap_or_default()>Duration::from_secs(7*24*3600);
        if bytes<=limit && count<=4096 && !expired {continue;}
        std::fs::remove_file(&path)?;bytes=bytes.saturating_sub(size);count=count.saturating_sub(1);
        if let Some(parent)=path.parent() {let _=std::fs::remove_dir(parent);}
    }
    ensure!(bytes<=limit,"Active export exceeds the disposable download cache quota");Ok(())
}

fn lease(path:&Path)->Result<std::fs::File> {
    let mut options=std::fs::OpenOptions::new();options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)] {use std::os::unix::fs::OpenOptionsExt;options.mode(0o600).custom_flags(libc::O_NOFOLLOW);}
    let file=options.open(path)?;ensure!(file.metadata()?.is_file(),"Replica lease is not a regular file");Ok(file)
}
pub(crate) fn replica_directory_lease(root:&Path)->Result<std::fs::File> {let file=lease(&root.join(".replica-admin.lock"))?;file.try_lock()?;Ok(file)}
pub(crate) fn replica_lease(path:&Path)->Result<std::fs::File> {lease(&path.with_extension("lock"))}
pub(crate) fn collect_replicas(target:&Path)->Result<()> {
    let managed=|p:&Path|p.extension().is_some_and(|e|e=="sqlite3") && p.file_stem().and_then(|s|s.to_str()).is_some_and(|s|s.len()==64 && s.bytes().all(|b|b.is_ascii_hexdigit()));
    if !managed(target) {return Ok(());}
    let mut candidates=files(target.parent().unwrap())?.into_iter().filter(|(p,_,_)|managed(p) && p!=target).collect::<Vec<_>>();
    candidates.sort_by_key(|(_,_,time)|*time);let mut count=candidates.len()+1;
    for (path,_,time) in candidates {
        if count<=4 && SystemTime::now().duration_since(time).unwrap_or_default()<=Duration::from_secs(7*24*3600) {continue;}
        let guard=replica_lease(&path)?;if guard.try_lock().is_err() {continue;}
        // Never unlink a live database or any authored store. The shared lease
        // spans all handles/jobs; remove its sidecar last, under the admin lock.
        for file in [path.clone(),PathBuf::from(format!("{}-wal",path.display())),PathBuf::from(format!("{}-shm",path.display()))] {
            match std::fs::remove_file(file) {Ok(())=>{},Err(e) if e.kind()==std::io::ErrorKind::NotFound=>{},Err(e)=>return Err(e.into())}
        }
        std::fs::remove_file(path.with_extension("lock"))?;count-=1;
    }
    ensure!(count<=4,"Four replica databases are active; close another client/account before opening more");
    if target.exists() {std::fs::OpenOptions::new().write(true).open(target)?.set_times(std::fs::FileTimes::new().set_modified(SystemTime::now()))?;}
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dormant_replica_collection_skips_live_handles_and_authored_state() {
        let root=tempfile::tempdir().unwrap();let store=crate::store::Store::open(root.path().into()).unwrap();store.put("pair","owned",&"retained").unwrap();
        let first=store.block_cache("first").unwrap();
        for n in 0..12 {drop(store.block_cache(&format!("pair-{n}")).unwrap());}
        let databases=files(&root.path().join("blocks")).unwrap().into_iter().filter(|(p,_,_)|p.extension().is_some_and(|e|e=="sqlite3")).count();assert_eq!(databases,4);
        assert!(!first.lineage().unwrap().is_empty());assert_eq!(store.get::<String>("pair","owned").unwrap(),"retained");
        let mut live=vec![first];for n in 0..3 {live.push(store.block_cache(&format!("live-{n}")).unwrap());}
        assert!(store.block_cache("too-many-live").is_err());drop(live);assert!(store.block_cache("after-close").is_ok());
    }
}
