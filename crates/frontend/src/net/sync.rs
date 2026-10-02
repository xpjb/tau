//! Explicit replica interests and resumable watch scheduling. Bodies commit to
//! the replica before credit is returned; these workers never own user intent.
use super::{ mailbox::EventSender, files, transfers::Transfers};
use crate::replica::{Cache, Plan};
use anyhow::{Result, ensure};
use std::{collections::{BTreeSet, HashMap}, sync::Arc, time::Duration};
use tau_net::{blocks::*, native::{Client, Priority}};
use tokio::{sync::{mpsc, watch}, task::JoinSet};

#[derive(Debug)]
pub enum ReplicaNotice {
    Changed(String),
    Failed { scope: String, error: anyhow::Error },
}

// Coalesced interests go straight from the UI to their owner, not through the
// WebSocket command queue followed by a second service command queue.
pub(super) struct Subscriptions {
    pub plans: watch::Sender<Vec<Plan>>,
    pub requests: mpsc::Sender<Request>,
    pub files: watch::Sender<Option<files::FileInterest>>,
    pub index: watch::Sender<Option<files::IndexInterest>>,
}
pub(super) struct Interests {
    plans: watch::Receiver<Vec<Plan>>,
    requests: mpsc::Receiver<Request>,
    files: watch::Receiver<Option<files::FileInterest>>,
    index: watch::Receiver<Option<files::IndexInterest>>,
}
pub(super) enum Request { Reset, History { scope:String,before:FeedPosition } }
pub(super) fn subscriptions() -> (Subscriptions, Interests) {
    let (plans,plan_rx)=watch::channel(vec![]);
    let (requests,request_rx)=mpsc::channel(8);
    let (files,file_rx)=watch::channel(None);
    let (index,index_rx)=watch::channel(None);
    (Subscriptions {plans,requests,files,index}, Interests {plans:plan_rx,requests:request_rx,files:file_rx,index:index_rx})
}

pub(super) struct Content {
    configuration: watch::Sender<Option<(BulkOffer,String)>>,
    transfers: Transfers,
    _tasks: JoinSet<()>,
}
impl Content {
    pub async fn start(cache: Cache, events: EventSender, notices: mpsc::Sender<ReplicaNotice>,
        updates: watch::Sender<Option<Arc<files::FileUpdate>>>, index_updates: watch::Sender<Option<Arc<files::IndexUpdate>>>, interests: Interests) -> Result<Self> {
        let client=Client::bind().await?;
        let wake=events.wake.clone();
        let (configuration,config_rx)=watch::channel(None);
        let (ready,lineage)=watch::channel(None);
        let mut tasks=JoinSet::new();
        tasks.spawn(files::watch_files(client.clone(),lineage.clone(),updates,wake.clone(),interests.files));
        tasks.spawn(files::watch_index(client.clone(),lineage.clone(),index_updates,wake.clone(),interests.index));
        let transfers=Transfers {cache:cache.clone(),client:client.clone(),ready:lineage,events};
        tasks.spawn(run(cache,client,wake,interests.requests,interests.plans,config_rx,notices,ready));
        Ok(Self {configuration,transfers,_tasks:tasks})
    }
    pub fn configure(&self, offer: BulkOffer, host: String) { self.configuration.send_replace(Some((offer,host))); }
    pub fn node_id(&self) -> String { self.transfers.client.node_id() }
    pub fn stats(&self) -> tau_net::native::Stats { self.transfers.client.stats() }
    pub fn transfers(&self) -> Transfers { self.transfers.clone() }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum Key { Feeds(String,Vec<Option<String>>,bool), BackgroundFeeds(Vec<(String,Option<String>)>), BackgroundBlock(String,String), Block(String,String,bool), History(String,Option<String>,FeedPosition,bool) }

fn background_batches(cache:&Cache, feeds:BTreeSet<(String,Option<String>)>) -> Result<Vec<Key>> {
    let mut batches=Vec::new();let mut keys=Vec::new();let mut requests=Vec::new();
    for (scope,parent) in feeds {
        let request=cache.feed_request(&scope,parent.as_deref(),None)?;
        let mut proposed=requests.clone();proposed.push(request.clone());
        if proposed.len()>16 || serde_json::to_vec(&BlockWatch::Feeds {requests:proposed})?.len()>MAX_BLOCK_HEADER_BYTES {
            ensure!(!keys.is_empty(),"Background feed exceeds request budget");
            batches.push(Key::BackgroundFeeds(std::mem::take(&mut keys)));requests.clear();
        }
        keys.push((scope,parent));requests.push(request);
    }
    if !keys.is_empty() {batches.push(Key::BackgroundFeeds(keys));}
    Ok(batches)
}
struct Watches(HashMap<Key,tokio::task::JoinHandle<()>>);
impl std::ops::Deref for Watches { type Target = HashMap<Key,tokio::task::JoinHandle<()>>; fn deref(&self) -> &Self::Target { &self.0 } }
impl std::ops::DerefMut for Watches { fn deref_mut(&mut self) -> &mut Self::Target { &mut self.0 } }
impl Drop for Watches { fn drop(&mut self) { for task in self.0.values() { task.abort(); } } }
async fn run(cache: Cache, client:Client, wake: crate::net::Wake, mut commands:mpsc::Receiver<Request>, mut plans:watch::Receiver<Vec<Plan>>, mut configuration:watch::Receiver<Option<(BulkOffer,String)>>, notices:mpsc::Sender<ReplicaNotice>, ready:watch::Sender<Option<String>>) {
    let mut jobs = Watches(HashMap::new());
    let mut plan = Vec::<Plan>::new();
    loop {
        tokio::select! {
            command=commands.recv()=>match command {
                None=>break,
                // Reset restarts current interests, never a separately queued stale plan.
                Some(Request::Reset)=>{for job in jobs.values() {job.abort();}jobs.clear();}
                Some(Request::History {scope,before})=>{
                    let key = Key::History(scope,None,before,false);
                    if jobs.get(&key).is_some_and(|task|task.is_finished()) {jobs.remove(&key);}
                    if !jobs.contains_key(&key) { jobs.insert(key.clone(),spawn_watch(key,client.clone(),cache.clone(),ready.subscribe(),notices.clone(),wake.clone())); }
                }
            },
            result=plans.changed()=>{if result.is_err() {break;}plan=plans.borrow_and_update().clone();}
            result=configuration.changed()=>{
                if result.is_err() {break;}
                let Some((offer,host))=configuration.borrow_and_update().clone() else {continue;};
                if let Err(error) = client.configure(&offer,&host).await {
                    let _ = notices.send(ReplicaNotice::Failed {scope:String::new(),error}).await; (wake)();
                } else if let Err(error) = cache.configure(&offer.lineage) {
                    let _ = notices.send(ReplicaNotice::Failed {scope:String::new(),error}).await; (wake)();
                } else {
                    if ready.borrow().as_ref().is_some_and(|old|old != &offer.lineage) { for task in jobs.values() {task.abort();} jobs.clear(); }
                    ready.send_if_modified(|current| { if current.as_ref()==Some(&offer.lineage) {false} else {*current=Some(offer.lineage);true} });
                }
            }
        }
        let mut desired=std::collections::HashSet::new();
        let mut background_feeds=BTreeSet::new();
        for p in &plan {
            // Batch headers, with a hard encoded request budget. Many expanded
            // cards must not consume every stream and starve their own bytes.
            let mut parents=vec![]; let mut requests=vec![];
            for parent in &p.parents {
                if p.background {background_feeds.insert((p.scope.clone(),parent.clone()));continue;}
                let Ok(request)=cache.feed_request(&p.scope,parent.as_deref(),None) else {continue;};
                let mut proposed=requests.clone();proposed.push(request.clone());
                if proposed.len()>16 || serde_json::to_vec(&BlockWatch::Feeds {requests:proposed}).map_or(true,|b|b.len()>MAX_BLOCK_HEADER_BYTES) {
                    desired.insert(Key::Feeds(p.scope.clone(),std::mem::take(&mut parents),p.background));requests.clear();
                }
                parents.push(parent.clone());requests.push(request);
            }
            if !parents.is_empty() {desired.insert(Key::Feeds(p.scope.clone(),parents,p.background));}
            for (id,head) in &p.blocks {
                // A complete body is not a download interest. In particular, a
                // delayed plan must not turn already-held text into a fetch of
                // its former queue ID after consumption. A changed/evicted
                // body gets a fresh plan from its metadata/cache update.
                if head.is_some_and(|(_,length,sealed,stored)|(p.background || sealed) && length==stored) {continue;}
                desired.insert(if p.background {Key::BackgroundBlock(p.scope.clone(),id.clone())}
                    else {Key::Block(p.scope.clone(),id.clone(),p.foreground.contains(id))});
            }
            for (parent,before) in &p.older {desired.insert(Key::History(p.scope.clone(),Some(parent.clone()),before.clone(),p.background));}
        }
        // Root/queue subscriptions are cheap. Batch them across chats instead
        // of occupying one long-lived stream per chat before any body can run.
        match background_batches(&cache,background_feeds) {
            Ok(batches)=>desired.extend(batches),
            Err(error)=>{let _=notices.send(ReplicaNotice::Failed {scope:String::new(),error}).await;(wake)();}
        }
        jobs.retain(|key,task| {
            let keep=if let Key::History(scope,None,_,_)=key {!task.is_finished() && plan.iter().any(|p|!p.background && &p.scope==scope)} else {desired.contains(key)};
            if !keep {task.abort();}keep
        });
        for key in desired {
            let completed=jobs.get(&key).is_some_and(|t|t.is_finished());
            let needed=match &key {
                Key::Block(scope,id,_)=>{
                    cache.needs_body(scope,id,false)
                }
                Key::BackgroundBlock(scope,id)=>{
                    cache.needs_body(scope,id,true)
                }
                _=>true,
            };
            if (!jobs.contains_key(&key) || completed) && needed {
                jobs.insert(key.clone(),spawn_watch(key,client.clone(),cache.clone(),ready.subscribe(),notices.clone(),wake.clone()));
            }
        }
    }
    for task in jobs.values() { task.abort(); }
    client.shutdown().await;
}

fn spawn_watch(key: Key, client: Client, cache: Cache, mut ready: watch::Receiver<Option<String>>, notices:mpsc::Sender<ReplicaNotice>, wake:crate::net::Wake) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let scope = match &key {
            Key::Feeds(scope,_,_) | Key::Block(scope,_,_) | Key::BackgroundBlock(scope,_) | Key::History(scope,_,_,_) => scope.clone(),
            Key::BackgroundFeeds(_)=>String::new(),
        };
        while ready.borrow().is_none() { if ready.changed().await.is_err() { return; } }
        let mut delay = 100;
        loop {
            let lineage = ready.borrow().clone().unwrap();
            let result = watch_once(&key,&client,&cache,&lineage,&notices,&wake).await;
            match result {
                Ok(true) => return,
                Ok(false) => continue, // Replica reset: rebuild interests from the new checkpoint.
                Err(error) => {
                    // Opt-in local context for lifecycle races; keep IDs out of
                    // the popup and never log message bodies or credentials.
                    if log::log_enabled!(target:"tau::content",log::Level::Debug) {
                        let state=if let Key::Block(scope,id,_)=&key {cache.body_checkpoint(scope,id)} else {None};
                        log::debug!(target:"tau::content","Content sync job {key:?}, current (header, cached prefix) {state:?}: {error:#}");
                    }
                    if notices.send(ReplicaNotice::Failed {scope:scope.clone(),error:error.context("Content sync")}).await.is_err() { return; }
                    (wake)();
                    tokio::time::sleep(Duration::from_millis(delay)).await;
                    delay = (delay*2).min(5000);
                }
            }
        }
    })
}

async fn watch_once(key: &Key, client: &Client, cache: &Cache, lineage:&str, notices: &mpsc::Sender<ReplicaNotice>, wake: &crate::net::Wake) -> Result<bool> {
    let epoch=cache.epoch();
    let request = match key {
        Key::Feeds(scope,parents,_) => BlockWatch::Feeds {requests:parents.iter().map(|parent|cache.feed_request(scope,parent.as_deref(),None)).collect::<Result<_>>()?},
        Key::BackgroundFeeds(feeds)=>BlockWatch::Feeds {requests:feeds.iter().map(|(scope,parent)|cache.feed_request(scope,parent.as_deref(),None)).collect::<Result<_>>()?},
        Key::History(scope,parent,before,_) => BlockWatch::Feed(cache.feed_request(scope,parent.as_deref(),Some(before.clone()))?),
        Key::Block(scope,id,_) => BlockWatch::Block(cache.block_request(scope,id)?),
        Key::BackgroundBlock(scope,id)=>{
            let mut request=cache.block_request(scope,id)?;
            // Catch up, then release the slot. Metadata announces later appends.
            request.follow=false;BlockWatch::Block(request)
        }
    };
    let background=matches!(key,Key::Block(_,_,false) | Key::Feeds(_,_,true) | Key::History(_,_,_,true) | Key::BackgroundFeeds(_) | Key::BackgroundBlock(..));
    let mut reader=client.read(request,if background {Priority::Background} else {Priority::Foreground}).await?;
    while let Some(update)=reader.next().await? {
        let Some(scope)=cache.apply(lineage,update,epoch).await? else {return Ok(true);};
        notices.send(ReplicaNotice::Changed(scope)).await?;(wake)();
        if cache.epoch()!=epoch {return Ok(false);}
    }
    Ok(true)
}

#[cfg(test)]
#[path = "../../tests/unit/net/sync/checkpoint.rs"]
mod checkpoint_tests;

#[cfg(test)]
#[path = "../../tests/unit/net/sync.rs"]
mod tests;
