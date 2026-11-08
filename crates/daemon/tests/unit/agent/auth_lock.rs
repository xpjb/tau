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
