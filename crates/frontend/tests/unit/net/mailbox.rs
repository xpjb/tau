use super::*;
use std::time::{Duration,Instant};
#[test]
fn sleeping_ui_coalesces_failed_acquisitions_without_erasing_disconnect_boundaries() {
    let (events,mut receiver)=channel(Arc::new(||{}));let now=Instant::now();
    assert!(events.send(Event::Ready {epoch:1,lineage:"first".into(),at:now}));
    assert!(events.send(Event::NotSent("intent".into(),"socket lost".into())));
    for attempt in 1..=1000 {
        assert!(events.send(Event::Connecting {attempt,at:now}));
        assert!(events.send(Event::Disconnected(format!("failure {attempt}"))));
        assert!(events.send(Event::RetryScheduled {at:now+Duration::from_secs(1)}));
    }
    assert!(events.send(Event::Ready {epoch:2,lineage:"second".into(),at:now}));
    assert!(events.send(Event::Disconnected("second socket lost".into())));
    assert!(matches!(receiver.try_recv().unwrap(),Event::Ready {epoch:1,lineage,..} if lineage=="first"));
    assert!(matches!(receiver.try_recv().unwrap(),Event::NotSent(id,_) if id=="intent"));
    assert!(matches!(receiver.try_recv().unwrap(),Event::Connecting {attempt:1000,..}));
    assert!(matches!(receiver.try_recv().unwrap(),Event::Disconnected(detail) if detail=="failure 1000"));
    assert!(matches!(receiver.try_recv().unwrap(),Event::RetryScheduled {..}));
    assert!(matches!(receiver.try_recv().unwrap(),Event::Ready {epoch:2,lineage,..} if lineage=="second"));
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
