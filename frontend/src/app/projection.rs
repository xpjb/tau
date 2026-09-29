//! Single existing model-to-row adapter. UI retention does not create another feed model.
use super::*;
pub(super) fn rows(model: &Controller, session: &str) -> Vec<Row> {
    let chat = &model.chats[session];
    let tools = Tools::new(chat.feed.events.values(), &chat.feed.parents)
        .with_lengths(&chat.feed.block_lengths)
        .with_states(&chat.feed.block_states);
    let events = chat
        .feed
        .events
        .values()
        .filter(|e| {
            let empty = e.attachment.is_none()
                && e.error_message.is_none()
                && !e.is_error
                && (e.kind == EventKind::Hidden
                    || matches!(e.kind, EventKind::Thinking | EventKind::Text) && e.text.is_empty());
            !empty && !(e.attachment.is_none() && tools.paired_result(e))
        })
        .collect::<Vec<_>>();
    let user_requests = events
        .iter()
        .filter(|e| e.role == EventRole::User && e.kind == EventKind::Text && e.attachment.is_none())
        .filter_map(|e| e.origin.request_id.as_deref())
        .collect::<std::collections::HashSet<_>>();
    let local_prompts = chat
        .local
        .pending
        .iter()
        .filter(|p| {
            matches!(p.request.command, ClientCommand::Prompt { .. }) && p.status != crate::store::Delivery::Rejected
        })
        .map(|p| (p.request.id.as_str(), p))
        .collect::<std::collections::HashMap<_, _>>();
    let mut shown_requests = std::collections::HashSet::new();
    let mut rows = vec![];
    let mut i = 0;
    while i < events.len() {
        let e = events[i];
        let detail = |e: &Event| {
            e.attachment.is_none()
                && (e.kind == EventKind::Thinking || e.kind == EventKind::Tool || e.role == EventRole::Tool)
        };
        if detail(e) {
            let start = i;
            while i < events.len() && detail(events[i]) {
                i += 1;
            }
            let group = &events[start..i];
            let details = tools.lines(group, &chat.local);
            rows.push(Row {
                block: None,
                key: format!("{session}/{}", details[0].key),
                details,
                header: false,
                title: String::new(),
                timestamp: clock::label(group.iter().find_map(|e| clock::event_ms(e))),
                sender: EventRole::Assistant,
                source: String::new(),
                user: false,
                error: false,
                actions: vec![(
                    "Copy message".into(),
                    ui::MenuChoice::CopyDetails(session.into(), group.iter().map(|e| e.id.clone()).collect()),
                )],
                attachment: None,
            });
            continue;
        }
        let user = e.role == EventRole::User;
        // The server owns ordering/identity; until its body is complete the
        // authored copy still owns the displayed text. Never turn it into
        // a Loading row or duplicate it at the end of the conversation.
        let local_text =
            (user && e.kind == EventKind::Text && e.attachment.is_none() && chat.feed.incomplete.contains(&e.id))
                .then(|| e.origin.request_id.as_deref().and_then(|id| local_prompts.get(id)))
                .flatten()
                .map(|p| p.text.as_str());
        let title = if user {
            if local_text.is_some() { "You · synchronizing" } else { "You" }.into()
        } else if let Some(error) = &e.error_message {
            format!("Assistant · {error}")
        } else {
            format!(
                "{}{}",
                if e.role == EventRole::Assistant { "Tau" } else { "System" },
                if e.phase == EventPhase::Live {
                    " · writing"
                } else if e.phase == EventPhase::Interrupted {
                    " · interrupted"
                } else {
                    ""
                }
            )
        };
        let mut actions = if let Some(text) = local_text {
            vec![("Copy text".into(), ui::MenuChoice::Copy(text.into()))]
        } else if chat.feed.incomplete.contains(&e.id) {
            vec![(
                "Fetch complete message to copy".into(),
                ui::MenuChoice::CopyDetails(session.into(), vec![e.id.clone()]),
            )]
        } else {
            vec![("Copy message".into(), ui::MenuChoice::Copy(e.text.clone()))]
        };
        if e.phase == EventPhase::Saved {
            actions.push(("Fork here".into(), ui::MenuChoice::Fork(e.entry_id.clone())));
        }
        let request = e
            .origin
            .request_id
            .as_deref()
            .filter(|id| user && e.kind == EventKind::Text && e.attachment.is_none() && shown_requests.insert(*id));
        rows.push(Row {
            block: Some(e.id.clone()),
            details: vec![],
            header: e.role == EventRole::System || e.is_error || e.error_message.is_some(),
            key: request.map_or_else(|| format!("{session}/{}", e.id), |id| format!("message:{session}:{id}")),
            title,
            timestamp: clock::label(clock::event_ms(e)),
            sender: if e.role == EventRole::Tool { EventRole::Assistant } else { e.role },
            source: if e.attachment.is_some() && chat.feed.incomplete.contains(&e.id) && e.text == "Loading…" {
                String::new()
            } else if user {
                literal(local_text.unwrap_or(&e.text))
            } else {
                e.text.clone()
            },
            user,
            error: e.is_error || e.error_message.is_some(),
            actions,
            attachment: e.attachment.clone().map(|a| (e.entry_id.clone(), a)),
        });
        i += 1;
    }
    for p in &chat.local.pending {
        let represented = user_requests.contains(p.request.id.as_str())
            || chat.feed.queue.requests.iter().any(|q| q.request_id == p.request.id);
        if matches!(p.request.command, ClientCommand::Prompt { .. })
            && p.status != crate::store::Delivery::Rejected
            && represented
        {
            continue;
        }
        let edit =
            matches!(&p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Edit { .. }, .. });
        let delete =
            matches!(&p.request.command, ClientCommand::QueueControl { operation: QueueOperation::Delete { .. }, .. });
        let control = matches!(&p.request.command, ClientCommand::QueueControl { .. } | ClientCommand::Abort { .. });
        let in_queue = chat.feed.queue.requests.iter().any(|q| match &p.request.command {
            ClientCommand::QueueControl {
                operation:
                    QueueOperation::Edit { request_id, revision, .. } | QueueOperation::Delete { request_id, revision },
                ..
            } => q.request_id == *request_id && q.revision <= revision.saturating_add(1),
            _ => false,
        });
        // The queue row owns an in-flight edit/delete. Never represent it as
        // a new user message; an unresolved control remains visible by itself
        // only if its target disappeared or it was explicitly rejected.
        if (edit || delete) && in_queue && !matches!(p.status, crate::store::Delivery::Rejected) {
            continue;
        }
        let mut actions = vec![("Copy text".into(), ui::MenuChoice::Copy(p.text.clone()))];
        if !control && matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed) {
            actions.push(("Restore draft".into(), ui::MenuChoice::Restore(p.request.id.clone())));
        }
        if matches!(p.request.command, ClientCommand::Prompt { .. })
            && matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed)
        {
            actions.push(("Retry saved message".into(), ui::MenuChoice::RetryPending(p.request.id.clone())));
        }
        actions.push(("Dismiss".into(), ui::MenuChoice::Dismiss(p.request.id.clone())));
        rows.push(Row {
            block: None,
            details: vec![],
            header: true,
            key: if matches!(p.request.command, ClientCommand::Prompt { .. }) && !represented {
                format!("message:{session}:{}", p.request.id)
            } else {
                format!("pending:{}", p.request.id)
            },
            title: if edit {
                format!("Queue edit · {}", p.status.label())
            } else if delete {
                format!("Queue delete · {}", p.status.label())
            } else if control {
                format!("Queue action · {}", p.status.label())
            } else {
                p.status.label().into()
            },
            timestamp: clock::label(p.started_at_ms),
            sender: if control { EventRole::System } else { EventRole::User },
            source: literal(&format!("{}{}", p.text, p.detail.as_ref().map(|s| format!("\n{s}")).unwrap_or_default())),
            user: !control,
            error: matches!(p.status, crate::store::Delivery::Rejected | crate::store::Delivery::Unconfirmed),
            actions,
            attachment: None,
        });
    }
    for (i, q) in chat.feed.queue.requests.iter().enumerate() {
        // Root and queue directories can arrive in either order during a
        // queue -> history move. The canonical user row wins that overlap.
        if user_requests.contains(q.request_id.as_str()) {
            continue;
        }
        let state = &chat.feed.queue;
        let moving = chat.feed.queue_transitions.contains_key(&q.request_id);
        let complete = !chat.feed.incomplete.contains(&format!("queued:{}", q.request_id));
        let local_text =
            (!complete).then(|| local_prompts.get(q.request_id.as_str())).flatten().map(|p| p.text.as_str());
        let mut actions =
            if complete { vec![("Copy message".into(), ui::MenuChoice::Copy(q.text.clone()))] } else { vec![] };
        if !moving && complete && state.capabilities.iter().any(|c| c == "queue_edit") {
            actions.push(("Edit".into(), ui::MenuChoice::EditQueue(q.request_id.clone(), q.revision, q.text.clone())));
        }
        if !moving && state.capabilities.iter().any(|c| c == "queue_delete") {
            actions.push((
                "Delete".into(),
                ui::MenuChoice::Queue(QueueOperation::Delete {
                    request_id: q.request_id.clone(),
                    revision: q.revision,
                }),
            ));
        }
        if state.available
            && chat.feed.queue_transitions.is_empty()
            && state.capabilities.iter().any(|c| c == "queue_run_prefix")
            && let Some(boundary) =
                state.boundaries.iter().find(|b| b.as_str() == "reasoning_checkpoint").or(state.boundaries.first())
        {
            actions.push((
                "Run through here".into(),
                ui::MenuChoice::Queue(QueueOperation::Prefix {
                    run_id: state.run_id.clone(),
                    requests: state.requests[..=i]
                        .iter()
                        .map(|q| QueueRef { request_id: q.request_id.clone(), revision: q.revision })
                        .collect(),
                    boundary: boundary.clone(),
                }),
            ));
        }
        let pending =
            chat.local.pending.iter().rev().find(|p| {
                let target = match &p.request.command {
                    ClientCommand::QueueControl {
                        operation:
                            QueueOperation::Edit { request_id, revision, .. }
                            | QueueOperation::Delete { request_id, revision },
                        ..
                    } => request_id == &q.request_id && q.revision <= revision.saturating_add(1),
                    _ => false,
                };
                target
                    && matches!(
                        p.status,
                        crate::store::Delivery::Sending
                            | crate::store::Delivery::Unconfirmed
                            | crate::store::Delivery::Accepted
                    )
            });
        let editing = pending.and_then(|p| match &p.request.command {
            ClientCommand::QueueControl { operation: QueueOperation::Edit { text, .. }, .. } => Some(text.as_str()),
            _ => None,
        });
        if let Some(text) = local_text {
            actions = vec![("Copy text".into(), ui::MenuChoice::Copy(text.into()))];
        }
        if pending.is_some() {
            // A second edit/delete using the old revision would race this
            // one. Wait for the durable receipt before offering actions.
            actions = vec![("Copy message".into(), ui::MenuChoice::Copy(editing.unwrap_or(&q.text).into()))];
        }
        rows.push(Row {
            block: None,
            details: vec![],
            header: true,
            key: format!("message:{session}:{}", q.request_id),
            title: if moving {
                "Synchronizing message".into()
            } else if let Some(p) = pending {
                format!(
                    "{} · {}",
                    if editing.is_some() { "Queue edit" } else { "Queue delete" },
                    if p.status == crate::store::Delivery::Unconfirmed {
                        "unconfirmed · not resent"
                    } else if p.status == crate::store::Delivery::Accepted {
                        "accepted · synchronizing…"
                    } else {
                        "saving…"
                    }
                )
            } else {
                format!("Queued{}", if state.paused { " · held" } else { "" })
            },
            timestamp: clock::label(q.timestamp_ms),
            sender: EventRole::User,
            source: literal(editing.or(local_text).unwrap_or(&q.text)),
            user: true,
            error: false,
            actions,
            attachment: None,
        });
    }
    rows
}
