//! Retained remote history. Nothing in this module is persisted or optimistically
//! invented: generations, ordering, queues and delivery receipts belong to taud.
use anyhow::{Result, ensure};
use std::collections::{BTreeMap, HashMap, HashSet};
use tau_protocol::*;

#[derive(Default)]
pub struct Feed {
    pub generation: String,
    pub sequence: u64,
    pub events: BTreeMap<u64, Event>,
    by_id: HashMap<String, u64>,
    previews:std::collections::VecDeque<(String,Vec<String>,usize)>,
    pub queue: QueueState,
    pub block_lengths: HashMap<String,u64>,
    pub incomplete: HashSet<String>,
    pub block_states: HashMap<String,String>,
    pub before: Option<u64>,
    pub synchronized: bool,
    pub loading: bool,
    pub opening: bool,
    pub revision: u64,
}

impl Feed {
    pub fn event(&self, id: &str) -> Option<&Event> {
        self.by_id.get(id).and_then(|n| self.events.get(n))
    }

    fn drop_preview(&mut self,ids:&[String]) {
        fn short(value:&mut Option<String>) {if let Some(s)=value {let mut n=s.len().min(64);while !s.is_char_boundary(n) {n-=1;}*s=s[..n].to_owned();}}
        for id in ids {
            if let Some(event)=self.by_id.get(id).and_then(|n|self.events.get_mut(n)) {
                event.text=if self.block_lengths.get(id).copied().unwrap_or(0)>0 && event.kind!=EventKind::Tool {"Loading…".into()} else {String::new()};
                short(&mut event.error_message);short(&mut event.tool_name);short(&mut event.stop_reason);
                if let Some(file)=&mut event.attachment {short(&mut file.caption);}
                self.incomplete.insert(id.clone());
            }
        }
    }
    pub(crate) fn native_view(&mut self,view:crate::blocks::View)->Result<Vec<String>> {
        let delivered=view.snapshot.events.iter().filter(|e|e.phase==EventPhase::Saved).filter_map(|e|e.origin.request_id.clone()).collect();
        if !view.partial {while let Some((_,ids,_))=self.previews.pop_front() {self.drop_preview(&ids);}}
        for (root,_,_) in &view.previews {
            if let Some(i)=self.previews.iter().position(|(old,_,_)|old==root) {let (_,ids,_)=self.previews.remove(i).unwrap();self.drop_preview(&ids);}
        }
        if view.partial {
            if view.queue_changed {self.incomplete.retain(|id|!id.starts_with("queued:"));}
            for event in &view.snapshot.events {self.incomplete.remove(&event.id);}
            self.native_patch(view.snapshot,view.queue_changed)?;
            self.block_lengths.extend(view.lengths);self.incomplete.extend(view.incomplete);self.block_states.extend(view.states);
        } else {
            self.generation.clear();self.snapshot(view.snapshot)?;self.block_lengths=view.lengths;self.incomplete=view.incomplete;self.block_states=view.states;
        }
        self.previews.extend(view.previews.into_iter().filter(|(_,_,bytes)|*bytes>0));
        let mut bytes=self.previews.iter().map(|(_,_,bytes)|bytes).sum::<usize>();
        while self.previews.len()>32 || bytes>8*1024*1024 {
            let (_,ids,size)=self.previews.pop_front().unwrap();bytes-=size;self.drop_preview(&ids);
        }
        Ok(delivered)
    }

    /// Sparse projection of already-verified native cache commits. Unlike the
    /// legacy wire adapter, body progress may share a directory cursor.
    pub(crate) fn native_patch(&mut self,snapshot:TranscriptSnapshot,queue:bool)->Result<()> {
        ensure!(self.generation==snapshot.generation && snapshot.sequence>=self.sequence,"Native projection generation changed");
        for event in &snapshot.events {
            if let Some(old)=self.event(&event.id) {ensure!(old.order==event.order,"Event order changed");}
            if let Some(old)=self.events.get(&event.order) {ensure!(old.id==event.id,"Event order collision");}
        }
        for event in snapshot.events {self.by_id.insert(event.id.clone(),event.order);self.events.insert(event.order,event);}
        self.sequence=snapshot.sequence;self.before=snapshot.before;if queue {self.queue=snapshot.queue;}
        self.revision+=1;Ok(())
    }
    /// Replace the authoritative tail. Older saved history survives only when
    /// the new cut overlaps it; a gap must never look like a continuous history.
    pub fn snapshot(&mut self, snapshot: TranscriptSnapshot) -> Result<bool> {
        if self.generation == snapshot.generation && snapshot.sequence < self.sequence {
            return Ok(false);
        }
        ensure!(
            !snapshot.generation.is_empty(),
            "Empty transcript generation"
        );
        let continuing = self.generation == snapshot.generation;
        let connected = continuing
            && snapshot.before.is_some_and(|before| {
                snapshot
                    .events
                    .iter()
                    .any(|e| e.order >= before && self.by_id.contains_key(&e.id))
            });
        let mut events = BTreeMap::new();
        if connected {
            for (n, e) in self.events.range(..snapshot.before.unwrap()) {
                if e.phase != EventPhase::Live {
                    events.insert(*n, e.clone());
                }
            }
        }
        let mut ids = HashSet::new();
        for event in snapshot.events {
            ensure!(
                !event.id.is_empty() && ids.insert(event.id.clone()),
                "Duplicate/empty event ID"
            );
            if continuing && let Some(old) = self.event(&event.id) {
                ensure!(old.order == event.order, "Event order changed");
            }
            ensure!(
                events.insert(event.order, event).is_none(),
                "Duplicate event order"
            );
        }
        let by_id: HashMap<_, _> = events.values().map(|e| (e.id.clone(), e.order)).collect();
        ensure!(by_id.len() == events.len(), "Duplicate event ID");
        self.by_id = by_id;
        self.events = events;
        self.generation = snapshot.generation;
        self.sequence = snapshot.sequence;
        self.queue = snapshot.queue;
        self.before = if connected {
            self.before.map(|n| n.min(snapshot.before.unwrap()))
        } else {
            snapshot.before
        };
        self.synchronized = true;
        self.loading = false;
        self.revision += 1;
        Ok(true)
    }

    /// Validate the entire patch before mutating anything. A gap or malformed
    /// delta invalidates the feed and triggers a fresh OpenSession, never replay.
    pub fn update(
        &mut self,
        generation: &str,
        sequence: u64,
        change: TranscriptChange,
    ) -> Result<bool> {
        if generation == self.generation && sequence <= self.sequence {
            return Ok(false);
        }
        let check = (|| -> Result<()> {
            ensure!(
                self.synchronized
                    && generation == self.generation
                    && self.sequence.checked_add(1) == Some(sequence),
                "Transcript gap"
            );
            let removed: HashSet<_> = change.removed.iter().collect();
            let mut ids = HashSet::new();
            let mut orders = HashSet::new();
            for event in &change.events {
                ensure!(
                    !event.id.is_empty() && ids.insert(&event.id) && orders.insert(event.order),
                    "Duplicate/empty event"
                );
                if let Some(old) = self.event(&event.id) {
                    ensure!(old.order == event.order, "Event order changed");
                }
                if let Some(owner) = self.events.get(&event.order) {
                    ensure!(
                        owner.id == event.id || removed.contains(&owner.id),
                        "Event order collision"
                    );
                }
            }
            if let Some(delta) = &change.delta {
                ensure!(
                    self.event(&delta.event_id)
                        .is_some_and(|e| e.phase == EventPhase::Live)
                        && !removed.contains(&delta.event_id),
                    "Delta without a live event"
                );
            }
            Ok(())
        })();
        if let Err(error) = check {
            self.synchronized = false;
            return Err(error);
        }
        for id in change.removed {
            if let Some(order) = self.by_id.remove(&id) {
                self.events.remove(&order);
            }
        }
        if let Some(delta) = change.delta {
            self.events
                .get_mut(&self.by_id[&delta.event_id])
                .unwrap()
                .text
                .push_str(&delta.text);
        }
        for event in change.events {
            self.by_id.insert(event.id.clone(), event.order);
            self.events.insert(event.order, event);
        }
        if let Some(queue) = change.queue {
            self.queue = queue;
        }
        self.sequence = sequence;
        self.revision += 1;
        Ok(true)
    }

    pub fn page(&mut self, generation: &str, cursor: u64, page: HistoryPage) -> Result<bool> {
        if generation != self.generation || Some(cursor) != self.before {
            return Ok(false);
        }
        ensure!(
            !page.events.is_empty() && page.before.is_none_or(|n| n < cursor),
            "History cursor did not advance"
        );
        let mut ids = HashSet::new();
        let mut previous = None;
        for event in &page.events {
            ensure!(
                !event.id.is_empty()
                    && ids.insert(&event.id)
                    && event.order < cursor
                    && page.before.is_none_or(|n| event.order >= n),
                "Invalid history event"
            );
            ensure!(
                previous.is_none_or(|n| n < event.order),
                "Unordered history"
            );
            previous = Some(event.order);
            if let Some(old) = self.event(&event.id) {
                ensure!(old.order == event.order, "History order changed");
            }
            if let Some(old) = self.events.get(&event.order) {
                ensure!(old.id == event.id, "History order collision");
            }
        }
        for event in page.events {
            self.by_id.insert(event.id.clone(), event.order);
            // A late history page must not replace a newer live update.
            self.events.entry(event.order).or_insert(event);
        }
        self.before = page.before;
        self.loading = false;
        self.revision += 1;
        Ok(true)
    }
}
