use super::*;
use std::time::Duration;
use tau_net::files::*;
#[test]
fn matching_is_latest_only_and_keeps_all_results_on_a_large_index() {
let paths=(0..50_000).map(|i|IndexedPath {path:format!("frontend/src/app/module_{i:05}.rs"),symlink:false}).collect::<Vec<_>>();
let reply=FileReply::Index {path:"/work".into(),revision:"a".repeat(64),base:None,entries:paths,removed:vec![],indexing:false,limited:false};
let index=Arc::new(PathIndex::apply(None,&reply).unwrap());
let (wake,wakes)=std::sync::mpsc::channel();
let matcher=Matcher::new(Arc::new(move || {let _=wake.send(());}));
let start=std::time::Instant::now();
let mut generation=0;
for query in ["zzzz", "module", "module4", "mdrs fnt"] {generation=matcher.query(index.clone(),query.into(),false);}
loop {
    wakes.recv_timeout(Duration::from_secs(10).saturating_sub(start.elapsed())).expect("Local matcher must wake an idle UI");
    if let Some(done)=matcher.take() && done.generation==generation {
        assert_eq!(done.rows.len(),50_000);assert!(Arc::ptr_eq(&index,&done.index));break;
    }
}
}
