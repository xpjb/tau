//! One coalesced viewer interest per UI. No filesystem bodies enter the replica
//! or control queues. Dropping/changing interest cancels its native stream.
use std::{sync::Arc, time::Duration};
use tau_protocol::files::*;
use tokio::sync::watch;

#[derive(Clone)]
pub struct Interest { pub generation: u64, pub request: FileRequest, pub document: Option<Arc<tau_code_viewer::Document>> }
pub struct Update {
    pub generation: u64, pub session: String, pub lineage: String,
    pub response: Result<FileReply, String>, pub document: Option<Arc<tau_code_viewer::Document>>,
}
pub(crate) struct Service { plans: watch::Sender<Option<Interest>>, task: tokio::task::JoinHandle<()> }
impl Service {
    pub fn start(mut client: watch::Receiver<Option<Arc<tau_transfer::blocks::Client>>>, mut ready: watch::Receiver<Option<String>>, updates: watch::Sender<Option<Arc<Update>>>, wake: crate::transport::Wake) -> Self {
        let (plans, mut interest) = watch::channel::<Option<Interest>>(None);
        let task = tokio::spawn(async move {
            loop {
                let plan = interest.borrow_and_update().clone();
                if plan.is_none() {updates.send_replace(None);}
                let endpoint = client.borrow_and_update().clone();
                let lineage = ready.borrow_and_update().clone();
                let read = async {
                    let (Some(plan), Some(endpoint), Some(lineage)) = (plan, endpoint, lineage) else { return std::future::pending::<()>().await; };
                    refresh(endpoint, plan, lineage, &updates, &wake).await;
                };
                tokio::select! {
                    _ = read => {},
                    changed = interest.changed() => { if changed.is_err() { break; } },
                    changed = client.changed() => { if changed.is_err() { break; } },
                    changed = ready.changed() => { if changed.is_err() { break; } },
                }
            }
        });
        Self { plans, task }
    }
    pub fn set(&self, interest: Option<Interest>) { self.plans.send_replace(interest); }
}
impl Drop for Service { fn drop(&mut self) { self.task.abort(); } }
async fn refresh(client: Arc<tau_transfer::blocks::Client>, plan: Interest, lineage: String, updates: &watch::Sender<Option<Arc<Update>>>, wake: &crate::transport::Wake) {
    let mut request = plan.request.clone();
    if matches!(request.operation, FileOperation::Search {..}) { tokio::time::sleep(Duration::from_millis(150)).await; }
    let mut document = plan.document.clone();
    let mut previous = None;
    loop {
        let result = client.files(request.clone()).await;
        let response = match result {
            Ok(FileReply::Text {text,..}) if text.len()>MAX_FILE_BYTES || text.bytes().filter(|&b|b==b'\n').count()>MAX_FILE_LINES => Err("File exceeds code preview limits".into()),
            Ok(FileReply::Text {ref revision,ref text,..}) if blake3::hash(text.as_bytes()).to_hex().as_str()!=revision => Err("File revision integrity check failed".into()),
            Ok(FileReply::Text { path, revision, text }) => {
                let old = document.clone(); let p = path.clone(); let r = revision.clone();
                match tokio::task::spawn_blocking(move || tau_code_viewer::Document::replace(old.as_deref(), p, r, text)).await {
                    Ok(doc) => {
                        document = Some(Arc::new(doc));
                        request.path = Some(path.clone()); request.operation = FileOperation::Open { revision: Some(revision.clone()) };
                        Ok(FileReply::Text { path, revision, text: String::new() })
                    }
                    Err(_) => Err("Could not prepare code preview".into()),
                }
            }
            Ok(reply) => Ok(reply),
            Err(error) => Err(error.to_string()),
        };
        // Unchanged files don't wake a sleeping renderer. An unchanged response
        // after an error still clears the stale/error state and restores selection.
        let unchanged = matches!(response, Ok(FileReply::Unchanged {..})) && previous.as_ref().is_some_and(Result::is_ok);
        if !unchanged && previous.as_ref() != Some(&response) {
            previous = Some(response.clone());
            updates.send_replace(Some(Arc::new(Update { generation: plan.generation, session: request.session_id.clone(), lineage: lineage.clone(), response, document: document.clone() })));
            (wake)();
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
