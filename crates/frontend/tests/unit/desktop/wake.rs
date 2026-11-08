use super::*;
use std::sync::{mpsc, atomic::AtomicUsize};
use std::time::Duration;

#[test]
fn burst_posts_one_os_wake_and_rearms_before_the_next_drain() {
    let posts = Arc::new(AtomicUsize::new(0));
    let count = posts.clone();
    let gate = WakeGate::new(Arc::new(move || { count.fetch_add(1, Ordering::SeqCst); }));
    for _ in 0..10_000 { gate.wake(); }
    assert_eq!(posts.load(Ordering::SeqCst), 1);
    assert_eq!(gate.begin_update(), (10_000, 1));
    // Completion arriving while update is running must not be swallowed.
    gate.wake();
    assert_eq!(posts.load(Ordering::SeqCst), 2);
    assert_eq!(gate.begin_update(), (1, 1));
    assert_eq!(gate.begin_update(), (0, 0));
}

#[test]
fn concurrent_producers_coalesce_without_losing_the_next_completion() {
    let (tx, rx) = mpsc::channel();
    let gate = WakeGate::new(Arc::new(move || { let _ = tx.send(()); }));
    std::thread::scope(|scope| {
        for _ in 0..8 {
            let gate = gate.clone();
            scope.spawn(move || { for _ in 0..1000 { gate.wake(); } });
        }
    });
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(rx.try_recv().is_err());
    assert_eq!(gate.begin_update(), (8000, 1));
    let next = gate.clone();
    std::thread::spawn(move || next.wake()).join().unwrap();
    rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(gate.begin_update(), (1, 1));
}
