use super::*;
use crate::state::StateStore;

#[tokio::test]
async fn wal_reads_do_not_wait_for_writes_and_each_read_keeps_one_committed_snapshot() {
    let root=tempfile::tempdir().unwrap();let state=StateStore::load(root.path().join("db")).await.unwrap();
    state.access(|db|{db.execute_batch("CREATE TABLE read_probe(value INTEGER); INSERT INTO read_probe VALUES(1);")?;Ok(())}).await.unwrap();
    let (started,ready)=tokio::sync::oneshot::channel();let (release,gate)=std::sync::mpsc::channel();
    let reader=tokio::spawn({let state=state.clone();async move {state.read(move |db|{
        let before:u32=db.query_row("SELECT value FROM read_probe",[],|r|r.get(0))?;
        let _=started.send(());gate.recv_timeout(Duration::from_secs(5))?;
        let after:u32=db.query_row("SELECT value FROM read_probe",[],|r|r.get(0))?;
        Ok((before,after))
    }).await.unwrap()}});ready.await.unwrap();
    tokio::time::timeout(Duration::from_secs(1),state.access(|db|{
        let tx=db.transaction()?;tx.execute("UPDATE read_probe SET value=2",[])?;tx.commit()?;Ok(())
    })).await.expect("The reader must not hold the writer mutex").unwrap();
    assert_eq!(state.read(|db|Ok(db.query_row("SELECT value FROM read_probe",[],|r|r.get::<_,u32>(0))?)).await.unwrap(),2);
    release.send(()).unwrap();assert_eq!(reader.await.unwrap(),(1,1),"A read never mixes two commits");

    let (started,ready)=tokio::sync::oneshot::channel();let (release,gate)=std::sync::mpsc::channel();
    let writer=tokio::spawn({let state=state.clone();async move {state.access(move |db|{
        let tx=db.transaction()?;tx.execute("UPDATE read_probe SET value=3",[])?;
        let _=started.send(());gate.recv_timeout(Duration::from_secs(5))?;
        drop(tx);Ok(()) // Roll back, including while readers are active.
    }).await.unwrap()}});ready.await.unwrap();
    let value=tokio::time::timeout(Duration::from_secs(1),state.read(|db|
        Ok(db.query_row("SELECT value FROM read_probe",[],|r|r.get::<_,u32>(0))?))).await
        .expect("A status read must not queue behind an uncommitted content write").unwrap();
    assert_eq!(value,2,"No dirty read");release.send(()).unwrap();writer.await.unwrap();
    assert!(state.read(|db|{db.execute("UPDATE read_probe SET value=4",[])?;Ok(())}).await.is_err(),"Read connections cannot mutate source state");
    assert_eq!(state.read(|db|Ok(db.query_row("SELECT value FROM read_probe",[],|r|r.get::<_,u32>(0))?)).await.unwrap(),2);
    assert_eq!(state.readers.permits.available_permits(),READERS,"Errors return leases too");
}

#[tokio::test]
async fn reader_pool_stays_bounded_and_cancellation_returns_its_connection_after_work_finishes() {
    let root=tempfile::tempdir().unwrap();let state=StateStore::load(root.path().join("db")).await.unwrap();
    let mut tasks=vec![];let mut releases=vec![];
    for _ in 0..READERS {
        let (started,ready)=tokio::sync::oneshot::channel();let (release,gate)=std::sync::mpsc::channel();
        tasks.push(tokio::spawn({let state=state.clone();async move {state.read(move |db|{
            let _:u32=db.query_row("SELECT 1",[],|r|r.get(0))?;
            let _=started.send(());gate.recv_timeout(Duration::from_secs(5))?;Ok(())
        }).await.unwrap()}}));ready.await.unwrap();releases.push(release);
    }
    assert_eq!(state.readers.permits.available_permits(),0);
    assert!(tokio::time::timeout(Duration::from_millis(20),state.read(|_|Ok(()))).await.is_err());
    tasks[0].abort();let _=(&mut tasks[0]).await;
    assert_eq!(state.readers.permits.available_permits(),0,"An aborted caller cannot lend a still-busy connection");
    for release in releases {release.send(()).unwrap();}
    for task in tasks.into_iter().skip(1) {task.await.unwrap();}
    tokio::time::timeout(Duration::from_secs(1),async {
        while state.readers.permits.available_permits()!=READERS {tokio::task::yield_now().await;}
    }).await.unwrap();
    state.read(|db|{db.query_row("SELECT 1",[],|_|Ok(()))?;Ok(())}).await.unwrap();
    assert_eq!(state.readers.idle.lock().unwrap().len(),READERS);
}
