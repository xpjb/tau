//! Nonblocking, bounded handoff to the UI. Superseded health and replicated
//! state are coalesced; the WebSocket reader never awaits an idle UI consumer.
use super::{Event,Wake};
use std::{collections::VecDeque,sync::{Arc,Mutex}};
use tau_net::ServerMessage;
use tokio::sync::Notify;

struct Queue {events:VecDeque<(Event,usize)>,bytes:usize,closed:bool}
struct Shared {queue:Mutex<Queue>,notify:Notify,senders:std::sync::atomic::AtomicUsize}
pub(super) struct EventSender {shared:Arc<Shared>,pub wake:Wake}
pub struct EventReceiver {shared:Arc<Shared>}

pub(super) fn channel(wake:Wake)->(EventSender,EventReceiver) {
    let shared=Arc::new(Shared {queue:Mutex::new(Queue {events:VecDeque::new(),bytes:0,closed:false}),notify:Notify::new(),senders:std::sync::atomic::AtomicUsize::new(1)});
    (EventSender {shared:shared.clone(),wake},EventReceiver {shared})
}
fn key(event:&Event)->Option<String> {
    match event {
        Event::Connecting { .. }=>Some("acquiring".into()),
        Event::RetryScheduled { .. }=>Some("acquisition-retry".into()),
        Event::Disconnected(_)=>Some("acquisition-failure".into()),
        Event::HeartbeatSent {epoch,..}=>Some(format!("sent:{epoch}")),
        Event::HeartbeatReply {epoch,..}=>Some(format!("reply:{epoch}")),
        Event::Metrics(_)=>Some("native-metrics".into()),
        Event::Download {key,..}=>Some(format!("download:{key}")),
        Event::Message(epoch,message)=>match message.as_ref() {
            ServerMessage::SessionState {session_id,..}=>Some(format!("state:{epoch}:{session_id}")),
            ServerMessage::Sessions {..}=>Some(format!("sessions:{epoch}")),
            ServerMessage::Projects {..}=>Some(format!("projects:{epoch}")),
            ServerMessage::Settings {..}=>Some(format!("settings:{epoch}")),
            ServerMessage::ModelCatalog {..}=>Some(format!("model-catalog:{epoch}")),
            ServerMessage::Commands {session_id,..}=>Some(format!("commands:{epoch}:{session_id}")),
            ServerMessage::ResyncRequired {session_id}=>Some(format!("resync:{epoch}:{session_id:?}")),
            _=>None,
        },
        _=>None,
    }
}
impl EventSender {
    pub fn send(&self,event:Event)->bool {
        let bytes=match &event {Event::Prepared {result,..}=>match result {Ok(text)=>text.len(),Err(error)=>error.len()},Event::Message(_,message)=>serde_json::to_vec(message).map_or(4096,|v|v.len()),_=>1024};
        self.send_sized(event,bytes)
    }
    // Descriptor size is mailbox accounting, not another kind of public event.
    pub(super) fn send_sized(&self,event:Event,bytes:usize)->bool {
        let mut queue=self.shared.queue.lock().unwrap();
        if queue.closed {return false;}
        // Fast refusal retries must not fill the mailbox while a phone's UI
        // sleeps. Coalesce acquisition progress only within one no-socket
        // episode: retaining the disconnect before each new Ready is essential
        // to fencing in-flight intents and checking their receipts on recovery.
        let after = if matches!(event, Event::Connecting { .. } | Event::RetryScheduled { .. } | Event::Disconnected(_)) {
            queue.events.iter().rposition(|(event,_)| matches!(event, Event::Ready { .. } | Event::Source(..) | Event::Fatal(_))).map_or(0,|at|at+1)
        } else {0};
        if let Some(key)=key(&event) && let Some(at)=queue.events.iter().enumerate().skip(after)
            .find_map(|(at,(old,_))|(self::key(old).as_ref()==Some(&key)).then_some(at)) {
            let revision=|event:&Event|match event {Event::Message(_,message)=>match message.as_ref() {ServerMessage::SessionState {revision,..}=>*revision,ServerMessage::ModelCatalog {catalog}=>catalog.revision,_=>0},_=>0};
            if revision(&queue.events[at].0)>revision(&event) {return true;}
            let (_,bytes)=queue.events.remove(at).unwrap();queue.bytes-=bytes;
        }
        // An overflowing durable lane fails closed instead of blocking probes.
        // Its receipts remain in the daemon journal for explicit reconciliation.
        if queue.events.len()>=512 || queue.bytes.saturating_add(bytes)>128*1024*1024 {
            let fatal=Event::Fatal("UI event backlog exceeded its limit. Reconnect to reconcile durable operations.".into());
            queue.events.push_back((fatal,0));queue.closed=true;drop(queue);self.shared.notify.notify_one();(self.wake)();return false;
        }
        queue.bytes+=bytes;queue.events.push_back((event,bytes));drop(queue);
        self.shared.notify.notify_one();(self.wake)();true
    }
}
impl EventReceiver {
    pub fn try_recv(&mut self)->Result<Event,tokio::sync::mpsc::error::TryRecvError> {
        let mut queue=self.shared.queue.lock().unwrap();
        if let Some((event,bytes))=queue.events.pop_front() {queue.bytes-=bytes;Ok(event)} else {Err(if queue.closed {tokio::sync::mpsc::error::TryRecvError::Disconnected} else {tokio::sync::mpsc::error::TryRecvError::Empty})}
    }
    pub async fn recv(&mut self)->Option<Event> {
        loop {let shared=self.shared.clone();let notified=shared.notify.notified();if let Ok(event)=self.try_recv() {return Some(event);}if self.shared.queue.lock().unwrap().closed {return None;}notified.await;}
    }
}
impl Drop for EventReceiver {fn drop(&mut self) {self.shared.queue.lock().unwrap().closed=true;}}

impl Clone for EventSender {fn clone(&self)->Self {self.shared.senders.fetch_add(1,std::sync::atomic::Ordering::Relaxed);Self {shared:self.shared.clone(),wake:self.wake.clone()}}}
impl Drop for EventSender {fn drop(&mut self) {if self.shared.senders.fetch_sub(1,std::sync::atomic::Ordering::AcqRel)==1 {self.shared.queue.lock().unwrap().closed=true;self.shared.notify.notify_one();}}}


#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration,Instant};
    #[test]
    fn sleeping_ui_coalesces_failed_acquisitions_without_erasing_disconnect_boundaries() {
        let (events,mut receiver)=channel(Arc::new(||{}));let now=Instant::now();
        assert!(events.send(Event::Ready {epoch:1,at:now}));
        assert!(events.send(Event::NotSent("intent".into(),"socket lost".into())));
        for attempt in 1..=1000 {
            assert!(events.send(Event::Connecting {attempt,at:now}));
            assert!(events.send(Event::Disconnected(format!("failure {attempt}"))));
            assert!(events.send(Event::RetryScheduled {at:now+Duration::from_secs(1)}));
        }
        assert!(events.send(Event::Ready {epoch:2,at:now}));
        assert!(events.send(Event::Disconnected("second socket lost".into())));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Ready {epoch:1,..}));
        assert!(matches!(receiver.try_recv().unwrap(),Event::NotSent(id,_) if id=="intent"));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Connecting {attempt:1000,..}));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Disconnected(detail) if detail=="failure 1000"));
        assert!(matches!(receiver.try_recv().unwrap(),Event::RetryScheduled {..}));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Ready {epoch:2,..}));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Disconnected(detail) if detail=="second socket lost"));
        assert!(receiver.try_recv().is_err());
    }
    #[tokio::test]
    async fn last_sender_closes_and_wakes_the_receiver_without_losing_queued_events() {
        use tokio::sync::mpsc::error::TryRecvError;
        let (events,mut receiver)=channel(Arc::new(||{}));
        let last=events.clone();drop(events);
        assert!(matches!(receiver.try_recv(),Err(TryRecvError::Empty)));
        assert!(last.send(Event::NotSent("id".into(),"offline".into())));
        assert!(matches!(receiver.recv().await,Some(Event::NotSent(..))));
        let waiting=tokio::spawn(async move {
            assert!(receiver.recv().await.is_none());
            assert!(matches!(receiver.try_recv(),Err(TryRecvError::Disconnected)));
        });
        tokio::task::yield_now().await;
        drop(last);
        tokio::time::timeout(Duration::from_secs(1),waiting).await.unwrap().unwrap();
    }

    #[test]
    fn download_progress_coalesces_to_the_final_result_for_a_sleeping_ui() {
        let (events,mut receiver)=channel(Arc::new(||{}));
        for transferred in 0..=1000 {
            assert!(events.send(Event::Download {key:"file".into(),path:"saved".into(),status:tau_net::TransferStatus {
                transferred,total:1000,network_bytes:transferred,done:transferred==1000,failure:None,
            }}));
        }
        assert!(matches!(receiver.try_recv().unwrap(),Event::Download {status,..} if status.done && status.transferred==1000));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn descriptor_accounting_enforces_the_budget_without_a_second_event_variant() {
        let (events,mut receiver)=channel(Arc::new(||{}));
        let message=||Event::Message(1,Box::new(ServerMessage::ResyncRequired {session_id:None}));
        assert!(events.send_sized(message(),128*1024*1024));
        // Replacing a coalesced value releases its old byte charge.
        assert!(events.send_sized(message(),10));
        assert!(events.send(Event::NotSent("id".into(),"offline".into())));
        assert!(!events.send_sized(Event::NotSent("large".into(),"".into()),128*1024*1024));
        assert!(!events.send(message()));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Message(..)));
        assert!(matches!(receiver.try_recv().unwrap(),Event::NotSent(..)));
        assert!(matches!(receiver.try_recv().unwrap(),Event::Fatal(_)));
        assert!(matches!(receiver.try_recv(),Err(tokio::sync::mpsc::error::TryRecvError::Disconnected)));
    }

}
