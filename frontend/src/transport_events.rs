//! Nonblocking, bounded handoff to the UI. Superseded health and replicated
//! state are coalesced; the WebSocket reader never awaits an idle UI consumer.
use super::{Event,Wake};
use std::{collections::VecDeque,sync::{Arc,Mutex}};
use tau_protocol::ServerMessage;
use tokio::sync::Notify;

struct Queue {events:VecDeque<(Event,usize)>,bytes:usize,closed:bool}
struct Shared {queue:Mutex<Queue>,notify:Notify,senders:std::sync::atomic::AtomicUsize}
pub(super) struct Events {shared:Arc<Shared>,pub wake:Wake}
pub struct EventReceiver {shared:Arc<Shared>}

pub(super) fn channel(wake:Wake)->(Events,EventReceiver) {
    let shared=Arc::new(Shared {queue:Mutex::new(Queue {events:VecDeque::new(),bytes:0,closed:false}),notify:Notify::new(),senders:std::sync::atomic::AtomicUsize::new(1)});
    (Events {shared:shared.clone(),wake},EventReceiver {shared})
}
fn key(event:&Event)->Option<String> {
    match event {
        Event::HeartbeatSent {epoch,..}=>Some(format!("sent:{epoch}")),
        Event::HeartbeatReply {epoch,..}=>Some(format!("reply:{epoch}")),
        Event::Message(epoch,message)=>match message.as_ref() {
            ServerMessage::SessionState {session_id,..}=>Some(format!("state:{epoch}:{session_id}")),
            ServerMessage::Sessions {..}=>Some(format!("sessions:{epoch}")),
            ServerMessage::Projects {..}=>Some(format!("projects:{epoch}")),
            ServerMessage::Settings {..}=>Some(format!("settings:{epoch}")),
            ServerMessage::Commands {session_id,..}=>Some(format!("commands:{epoch}:{session_id}")),
            ServerMessage::ResyncRequired {session_id}=>Some(format!("resync:{epoch}:{session_id:?}")),
            _=>None,
        },
        _=>None,
    }
}
impl Events {
    pub async fn send(&self,event:Event)->bool {self.send_now(event)}
    pub fn send_now(&self,event:Event)->bool {
        let bytes=match &event {Event::Message(_,message)=>serde_json::to_vec(message).map_or(4096,|v|v.len()),_=>1024};
        let mut queue=self.shared.queue.lock().unwrap();
        if queue.closed {return false;}
        if let Some(key)=key(&event) && let Some(at)=queue.events.iter().position(|(old,_)|self::key(old).as_ref()==Some(&key)) {
            let (_,bytes)=queue.events.remove(at).unwrap();queue.bytes-=bytes;
        }
        // An overflowing durable lane fails closed instead of blocking probes.
        // Its receipts remain in the daemon journal for explicit reconciliation.
        if queue.events.len()>=512 || queue.bytes.saturating_add(bytes)>128*1024*1024 {
            let fatal=Event::Fatal("UI event backlog exceeded its limit. Reconnect to reconcile durable operations.".into());
            queue.events.push_back((fatal,0));drop(queue);self.shared.notify.notify_one();(self.wake)();return false;
        }
        queue.bytes+=bytes;queue.events.push_back((event,bytes));drop(queue);
        self.shared.notify.notify_one();(self.wake)();true
    }
}
impl EventReceiver {
    pub fn try_recv(&mut self)->Result<Event,tokio::sync::mpsc::error::TryRecvError> {
        let mut queue=self.shared.queue.lock().unwrap();
        if let Some((event,bytes))=queue.events.pop_front() {queue.bytes-=bytes;Ok(event)} else {Err(tokio::sync::mpsc::error::TryRecvError::Empty)}
    }
    pub async fn recv(&mut self)->Option<Event> {
        loop {let shared=self.shared.clone();let notified=shared.notify.notified();if let Ok(event)=self.try_recv() {return Some(event);}if self.shared.queue.lock().unwrap().closed {return None;}notified.await;}
    }
}
impl Drop for EventReceiver {fn drop(&mut self) {self.shared.queue.lock().unwrap().closed=true;}}

impl Clone for Events {fn clone(&self)->Self {self.shared.senders.fetch_add(1,std::sync::atomic::Ordering::Relaxed);Self {shared:self.shared.clone(),wake:self.wake.clone()}}}
impl Drop for Events {fn drop(&mut self) {if self.shared.senders.fetch_sub(1,std::sync::atomic::Ordering::AcqRel)==1 {self.shared.queue.lock().unwrap().closed=true;self.shared.notify.notify_waiters();}}}
