//! Transitional presentation consumer of model-owned message identities; removed in 041.
use super::*;
use crate::feed::MessageBody;
pub(super) fn rows(model: &Controller, session: &str) -> Vec<Row> {
    let chat = &model.chats[session];
    let feed = &chat.feed;
    let tools = Tools::new(feed.events.values(), &feed.parents).with_bodies(&feed.bodies).with_states(&feed.block_states);
    let mut rows = vec![];
    let mut i = 0;
    while i < feed.order.len() {
        let id = &feed.order[i];
        let m = &feed.messages[id];
        let event = m.event.as_deref().and_then(|id| feed.event(id));
        let detail = |e: &Event| e.attachment.is_none() && (matches!(e.kind, EventKind::Thinking | EventKind::Tool) || e.role == EventRole::Tool);
        if event.is_some_and(detail) {
            let group = feed.order[i..].iter().map(|id| &feed.messages[id])
                .map_while(|m| m.event.as_deref().and_then(|id| feed.event(id)).filter(|e| detail(e))).collect::<Vec<_>>();
            i += group.len();
            let details = tools.lines(&group, &chat.local);
            rows.push(Row {
                block: None, key: format!("{session}/{}", details[0].key), details,
                header: false, title: String::new(), timestamp: clock::label(group.iter().find_map(|e| clock::event_ms(e))),
                sender: EventRole::Assistant, source: String::new(), user: false, error: false,
                actions: vec![("Copy message".into(), ui::MenuChoice::CopyDetails(session.into(), group.iter().map(|e| e.id.clone()).collect()))], attachment: None,
            });
            continue;
        }
        i += 1;
        let pending = m.intent.as_deref().and_then(|id| chat.local.pending.iter().find(|p| p.request.id == id));
        let text = m.text(feed, &chat.local);
        let mut row = Row {
            block: m.event.clone(), key: id.key(session), details: vec![], header: true, title: String::new(),
            timestamp: String::new(), sender: EventRole::User, source: literal(text), user: true, error: false, actions: vec![], attachment: None,
        };
        if let Some(e) = event {
            row.user = e.role == EventRole::User;
            row.sender = if e.role == EventRole::Tool { EventRole::Assistant } else { e.role };
            row.header = e.role == EventRole::System || e.is_error || e.error_message.is_some();
            row.error = e.is_error || e.error_message.is_some();
            row.timestamp = clock::label(clock::event_ms(e));
            row.title = if row.user { if pending.is_some() { "You · synchronizing" } else { "You" }.into() }
                else if let Some(error) = &e.error_message { format!("Assistant · {error}") }
                else { format!("{}{}", if e.role == EventRole::Assistant { "Tau" } else { "System" },
                    if e.phase == EventPhase::Live { " · writing" } else if e.phase == EventPhase::Interrupted { " · interrupted" } else { "" }) };
            row.source = if row.user { literal(text) } else { text.into() };
            if !matches!(m.body, MessageBody::Local(_)) {
                if e.attachment.is_none() && feed.bodies.get(&e.id).is_some_and(|b| b.missing()) { row.source = "Loading…".into(); }
                if feed.bodies.get(&e.id).is_some_and(|b| b.limited) { row.source.push_str("\n\n[Preview limited. Fetch the complete message with Copy.]"); }
            }
            row.actions = if matches!(m.body, MessageBody::Local(_)) { vec![("Copy text".into(), ui::MenuChoice::Copy(text.into()))] }
                else if feed.incomplete.contains(&e.id) { vec![("Fetch complete message to copy".into(), ui::MenuChoice::CopyDetails(session.into(), vec![e.id.clone()]))] }
                else { vec![("Copy message".into(), ui::MenuChoice::Copy(text.into()))] };
            if e.phase == EventPhase::Saved { row.actions.push(("Fork here".into(), ui::MenuChoice::Fork(e.entry_id.clone()))); }
            row.attachment = e.attachment.clone().map(|a| (e.entry_id.clone(), a));
        } else if let Some(qid) = &m.queue {
            let (index, q) = feed.queue.requests.iter().enumerate().find(|(_, q)| &q.request_id == qid).unwrap();
            let state = &feed.queue;
            let moving = feed.queue_transitions.contains_key(qid);
            let complete = !feed.incomplete.contains(&format!("queued:{qid}"));
            let control = pending.filter(|p| matches!(p.request.command, ClientCommand::QueueControl { .. }));
            let editing = control.is_some_and(|p| matches!(p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. }));
            row.timestamp = clock::label(q.timestamp_ms);
            row.title = if moving { "Synchronizing message".into() }
                else if let Some(p) = control { format!("{} · {}", if editing { "Queue edit" } else { "Queue delete" },
                    if p.status == crate::store::Delivery::Unconfirmed { "unconfirmed · not resent" }
                    else if p.status == crate::store::Delivery::Accepted { "accepted · synchronizing…" } else { "saving…" }) }
                else { format!("Queued{}", if state.paused { " · held" } else { "" }) };
            if complete { row.actions.push(("Copy message".into(), ui::MenuChoice::Copy(text.into()))); }
            if !moving && complete && state.capabilities.iter().any(|c| c == "queue_edit") {
                row.actions.push(("Edit".into(), ui::MenuChoice::EditQueue(qid.clone(), q.revision, q.text.clone())));
            }
            if !moving && state.capabilities.iter().any(|c| c == "queue_delete") {
                row.actions.push(("Delete".into(), ui::MenuChoice::Queue(QueueOperation::Delete { request_id: qid.clone(), revision: q.revision })));
            }
            if state.available && feed.queue_transitions.is_empty() && state.capabilities.iter().any(|c| c == "queue_run_prefix")
                && let Some(boundary) = state.boundaries.iter().find(|b| b.as_str() == "reasoning_checkpoint").or(state.boundaries.first()) {
                row.actions.push(("Run through here".into(), ui::MenuChoice::Queue(QueueOperation::Prefix {
                    run_id: state.run_id.clone(), requests: state.requests[..=index].iter().map(|q| QueueRef { request_id: q.request_id.clone(), revision: q.revision }).collect(), boundary: boundary.clone(),
                })));
            }
            if pending.is_some() { row.actions = vec![("Copy text".into(), ui::MenuChoice::Copy(text.into()))]; }
        } else if let Some(p) = pending {
            let edit = matches!(p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. });
            let delete = matches!(p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Delete { .. }, .. });
            let control = matches!(p.request.command, ClientCommand::QueueControl { .. } | ClientCommand::Abort { .. });
            row.user = !control;
            row.sender = if control { EventRole::System } else { EventRole::User };
            row.error = matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed);
            row.timestamp = clock::label(p.started_at_ms);
            row.title = format!("{}{}", if edit { "Queue edit · " } else if delete { "Queue delete · " } else if control { "Queue action · " } else { "" }, p.status.label());
            row.source = literal(&format!("{}{}", text, p.detail.as_ref().map(|d| format!("\n{d}")).unwrap_or_default()));
            row.actions.push(("Copy text".into(), ui::MenuChoice::Copy(text.into())));
            if !control && row.error { row.actions.push(("Restore draft".into(), ui::MenuChoice::Restore(p.request.id.clone()))); }
            if matches!(p.request.command, ClientCommand::Prompt { .. }) && row.error { row.actions.push(("Retry saved message".into(), ui::MenuChoice::RetryPending(p.request.id.clone()))); }
            row.actions.push(("Dismiss".into(), ui::MenuChoice::Dismiss(p.request.id.clone())));
        }
        rows.push(row);
    }
    rows
}
