//! The real event mailbox and subscription channels, with no sockets or server.
//! Keep outbound receivers alive so normal Controller::poll takes its real path.
use super::*;

pub(crate) struct Mailbox {
    events: EventSender,
    _commands: mpsc::Receiver<Command>,
    _interests: sync::Interests,
    _replicas: mpsc::Sender<ReplicaNotice>,
    _files: tokio::sync::watch::Sender<Option<Arc<files::FileUpdate>>>,
    _index: tokio::sync::watch::Sender<Option<Arc<files::IndexUpdate>>>,
}
impl Mailbox {
    pub fn new(wake: Wake) -> (Network, Self) {
        let (events, incoming) = mailbox::channel(wake);
        let (subscriptions, interests) = sync::subscriptions();
        let (tx, commands) = mpsc::channel(64);
        let (notices, replicas) = mpsc::channel(32);
        let (file_tx, files) = tokio::sync::watch::channel(None);
        let (index_tx, file_index) = tokio::sync::watch::channel(None);
        (Network { tx, subscriptions, events: incoming, replicas, files, file_index },
            Self { events, _commands: commands, _interests: interests, _replicas: notices,
                _files: file_tx, _index: index_tx })
    }
    pub fn send(&self, event: Event) { assert!(self.events.send(event)); }
}
