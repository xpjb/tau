//! Retained remote history. Nothing in this module is persisted or optimistically
//! invented: generations, ordering, queues and delivery receipts belong to taud.
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, HashMap, HashSet};
use tau_net::*;

/// Logical identities and body ownership, not UI descriptions. Ordered entries
/// refer to the durable outbox or verified feed; they never duplicate authored bytes.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MessageId { Event(String), Request(String), Intent(String) }
impl MessageId {
    pub fn key(&self, scope: &str) -> String {
        match self {
            Self::Event(id) => format!("{scope}/{id}"),
            Self::Request(id) => format!("message:{scope}:{id}"),
            Self::Intent(id) => format!("pending:{id}"),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MessageBody { Remote(String), Queue(String), Local(String) }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub event: Option<String>,
    pub queue: Option<String>,
    pub intent: Option<String>,
    pub body: MessageBody,
}
impl Message {
    pub fn text<'a>(&self, feed: &'a Feed, local: &'a crate::store::LocalChat) -> &'a str {
        match &self.body {
            MessageBody::Remote(id) => feed.event(id).map(|e| e.text.as_str()),
            MessageBody::Queue(id) => feed.queue.requests.iter().find(|q| &q.request_id == id).map(|q| q.text.as_str()),
            MessageBody::Local(id) => local.pending.iter().find(|p| &p.request.id == id).map(|p| p.text.as_str()),
        }.unwrap_or("")
    }
}

#[derive(Default)]
pub struct Feed {
    pub order: Vec<MessageId>,
    pub messages: HashMap<MessageId, Message>,
    pub generation: String,
    pub sequence: u64,
    pub events: BTreeMap<u64, Event>,
    by_id: HashMap<String, u64>,
    previews:std::collections::VecDeque<(String,Vec<String>,usize)>,
    pub queue: QueueState,
    pub queue_transitions: HashMap<String,u64>, // Display-only rows awaiting the root cursor.
    pub bodies: HashMap<String,crate::replica::Body>,
    pub incomplete: HashSet<String>,
    pub block_states: HashMap<String,blocks::ToolState>,
    pub parents: HashMap<String,String>,
    pub children: HashMap<String, Vec<String>>,
    pub before: Option<u64>,
    pub synchronized: bool,
    pub loading: bool,
    pub opening: bool,
    pub revision: u64,
}

impl Feed {
    /// Install display ownership at the model boundary, without retiring intent
    /// or executing anything. Receipt/content evidence remains in LocalChat.
    pub fn reconcile(&mut self, local: &crate::store::LocalChat) {
        use crate::store::Delivery;
        let prompts = local.pending.iter().filter(|p|
            matches!(p.request.command, ClientCommand::Prompt { .. }) && p.status != Delivery::Rejected)
            .map(|p| (p.request.id.as_str(), p)).collect::<HashMap<_, _>>();
        let mut order = Vec::new();
        let mut messages = HashMap::new();
        let mut requests = HashSet::new();
        let mut insert = |id: MessageId, message: Message| { order.push(id); messages.insert(order.last().unwrap().clone(), message); };
        for e in self.events.values() {
            if e.attachment.is_none() && e.error_message.is_none() && !e.is_error
                && (e.kind == EventKind::Hidden || matches!(e.kind, EventKind::Thinking | EventKind::Text)
                    && e.text.is_empty() && !self.incomplete.contains(&e.id)) { continue; }
            if e.attachment.is_none() && e.role == EventRole::Tool && self.parents.contains_key(&e.id) { continue; }
            let request = e.origin.request_id.as_deref()
                .filter(|_| e.role == EventRole::User && e.kind == EventKind::Text && e.attachment.is_none());
            if request.is_some_and(|id| !requests.insert(id.to_owned())) { continue; }
            let intent = request.filter(|_| self.incomplete.contains(&e.id)).and_then(|id| prompts.get(id));
            insert(request.map_or_else(|| MessageId::Event(e.id.clone()), |id| MessageId::Request(id.into())), Message {
                event: Some(e.id.clone()), queue: None, intent: intent.map(|p| p.request.id.clone()),
                body: intent.map_or_else(|| MessageBody::Remote(e.id.clone()), |p| MessageBody::Local(p.request.id.clone())),
            });
        }
        let represented = |id: &str| requests.contains(id) || self.queue.requests.iter().any(|q| q.request_id == id);
        let targets = |p: &crate::store::Pending, q: &QueuedRequest| match &p.request.command {
            ClientCommand::QueueControl { operation: QueueOperation::Edit { request_id, revision, .. }
                | QueueOperation::Delete { request_id, revision }, .. } => request_id == &q.request_id && q.revision <= revision.saturating_add(1),
            _ => false,
        };
        for p in &local.pending {
            let prompt = matches!(p.request.command, ClientCommand::Prompt { .. });
            if p.status != Delivery::Rejected && (prompt && represented(&p.request.id)
                || self.queue.requests.iter().any(|q| targets(p, q))) { continue; }
            let id = if prompt && !represented(&p.request.id) { MessageId::Request(p.request.id.clone()) }
                else { MessageId::Intent(p.request.id.clone()) };
            insert(id, Message { event: None, queue: None, intent: Some(p.request.id.clone()), body: MessageBody::Local(p.request.id.clone()) });
        }
        for q in &self.queue.requests {
            if requests.contains(&q.request_id) { continue; }
            let pending = local.pending.iter().rev().find(|p| targets(p, q)
                && matches!(p.status, Delivery::Sending | Delivery::Unconfirmed | Delivery::Accepted));
            let original = self.incomplete.contains(&format!("queued:{}", q.request_id))
                .then(|| prompts.get(q.request_id.as_str())).flatten().copied();
            let editing = pending.filter(|p| matches!(p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. }));
            insert(MessageId::Request(q.request_id.clone()), Message {
                event: None, queue: Some(q.request_id.clone()), intent: pending.or(original).map(|p| p.request.id.clone()),
                body: editing.or(original).map_or_else(|| MessageBody::Queue(q.request_id.clone()), |p| MessageBody::Local(p.request.id.clone())),
            });
        }
        self.children.clear();
        for e in self.events.values() {
            if let Some(parent) = self.parents.get(&e.id) { self.children.entry(parent.clone()).or_default().push(e.id.clone()); }
        }
        self.order = order;
        self.messages = messages;
        self.revision += 1;
    }

    /// Conservative decoded-view accounting, not a process-RSS promise. Headers
    /// are <=4 KiB each; preview payloads and full queued text are counted too.
    pub(crate) fn resident_bytes(&self) -> usize {
        self.previews.iter().map(|(_,_,bytes)|bytes).sum::<usize>()
            + self.events.len() * tau_net::blocks::MAX_BLOCK_HEADER_BYTES
            + self.queue.requests.iter().map(|q|q.text.len()+1024).sum::<usize>()
    }

    pub(crate) fn retained_roots(&self) -> std::collections::BTreeSet<String> {
        self.previews.iter().map(|(root,_,_)| root.clone()).collect()
    }

    pub fn event(&self, id: &str) -> Option<&Event> {
        self.by_id.get(id).and_then(|n| self.events.get(n))
    }

    fn drop_preview(&mut self,ids:&[String]) {
        fn short(value:&mut Option<String>) {if let Some(s)=value {let mut n=s.len().min(64);while !s.is_char_boundary(n) {n-=1;}*s=s[..n].to_owned();}}
        for id in ids {
            if let Some(event)=self.by_id.get(id).and_then(|n|self.events.get_mut(n)) {
                event.text.clear();
                if let Some(body) = self.bodies.get_mut(id) { body.resident = 0; body.limited = false; }
                short(&mut event.error_message);short(&mut event.tool_name);short(&mut event.stop_reason);
                if let Some(file)=&mut event.attachment {short(&mut file.caption);}
                self.incomplete.insert(id.clone());
            }
        }
    }
    pub(crate) fn native_view(&mut self,mut view:crate::replica::View)->Result<Vec<String>> {
        let mut ids = HashSet::new(); let mut orders = HashSet::new();
        ensure!(view.events.iter().all(|e| !e.id.is_empty() && ids.insert(&e.id) && orders.insert(e.order)), "Duplicate native event identity/order");
        if self.generation == view.generation {
            if view.queue_changed {
                for (i,q) in self.queue.requests.iter().enumerate() {
                    if !view.queue.requests.iter().any(|next|next.request_id==q.request_id)
                        && let Some(revision) = view.queue_removals.get(&q.request_id) {
                        view.queue.requests.insert(i.min(view.queue.requests.len()),q.clone());
                        self.queue_transitions.insert(q.request_id.clone(),*revision);
                        let id=format!("queued:{}",q.request_id);
                        if let Some(body) = self.bodies.get(&id) { view.bodies.insert(id.clone(), body.clone()); }
                        if self.incomplete.contains(&id) { view.incomplete.insert(id); }
                    }
                }
            } else {
                self.queue.requests.retain(|q| self.queue_transitions.get(&q.request_id).is_none_or(|revision| *revision > view.sequence));
            }
            self.queue_transitions.retain(|_,revision| *revision > view.sequence);
        } else { self.queue_transitions.clear(); }
        let delivered=view.delivered.clone();
        if !view.partial {while let Some((_,ids,_))=self.previews.pop_front() {self.drop_preview(&ids);}}
        for (root,_,_) in &view.previews {
            if let Some(i)=self.previews.iter().position(|(old,_,_)|old==root) {let (_,ids,_)=self.previews.remove(i).unwrap();self.drop_preview(&ids);}
        }
        if view.partial {
            if view.queue_changed {self.incomplete.retain(|id|!id.starts_with("queued:"));self.bodies.retain(|id,_|!id.starts_with("queued:"));}
            for event in &view.events {self.incomplete.remove(&event.id);self.parents.remove(&event.id);self.block_states.remove(&event.id);}
            ensure!(self.generation == view.generation && view.sequence >= self.sequence, "Native generation changed");
            for event in &view.events {
                if let Some(old) = self.event(&event.id) { ensure!(old.order == event.order, "Event order changed"); }
                if let Some(old) = self.events.get(&event.order) { ensure!(old.id == event.id, "Event order collision"); }
            }
            self.bodies.extend(view.bodies);self.incomplete.extend(view.incomplete);self.block_states.extend(view.states);self.parents.extend(view.parents);
        } else {
            self.events.clear(); self.by_id.clear(); self.loading = false; self.synchronized = true;
            self.bodies=view.bodies;self.incomplete=view.incomplete;self.block_states=view.states;self.parents=view.parents;
        }
        for event in view.events { self.by_id.insert(event.id.clone(), event.order); self.events.insert(event.order, event); }
        self.generation = view.generation; self.sequence = view.sequence; self.before = view.before;
        if view.queue_changed { self.queue = view.queue; }
        self.revision += 1;
        self.previews.extend(view.previews.into_iter().filter(|(_,_,bytes)|*bytes>0));
        let mut bytes=self.previews.iter().map(|(_,_,bytes)|bytes).sum::<usize>();
        while self.previews.len()>128 || bytes>8*1024*1024 {
            let (_,ids,size)=self.previews.pop_front().unwrap();bytes-=size;self.drop_preview(&ids);
        }
        Ok(delivered)
    }
}

use crate::store::LocalChat;


pub(crate) fn detail_group_state(group: &[&Event], local: &LocalChat) -> (String,bool) {
    let key = group.iter().map(|e|format!("details:{}",e.id)).find(|key|local.expansion.contains_key(key))
        .unwrap_or_else(||format!("details:{}",group[0].id));
    let open = local.expansion.get(&key).copied().unwrap_or_else(||local.details_default || group.iter().any(|e|local.expanded.contains(&e.id)));
    (key,open)
}

/// The same disclosure grouping used by rendering, operating on root headers only.
pub(crate) fn expanded_details(events: &[Event], local: &LocalChat) -> HashSet<String> {
    let visible = events.iter().filter(|e| {
        let empty = e.attachment.is_none() && e.error_message.is_none() && !e.is_error &&
            (e.kind == EventKind::Hidden || matches!(e.kind,EventKind::Thinking|EventKind::Text) && e.text.is_empty());
        !empty
    }).collect::<Vec<_>>();
    let detail = |e:&Event|e.attachment.is_none() && (matches!(e.kind,EventKind::Thinking|EventKind::Tool) || e.role == EventRole::Tool);
    let mut open = HashSet::new(); let mut i=0;
    while i < visible.len() {
        if !detail(visible[i]) {i+=1;continue;}
        let start=i; while i<visible.len() && detail(visible[i]) {i+=1;}
        let group=&visible[start..i];
        if detail_group_state(group,local).1 {open.extend(group.iter().map(|e|e.id.clone()));}
    }
    open
}
