//! Tau 1's disclosure hierarchy: Details → tool → large Input/Output sections.
//! Tool membership uses native parents. Provider call IDs remain opaque metadata.
use crate::store::LocalChat;
use std::collections::{HashMap, HashSet};
use tau_protocol::{Event, EventKind, EventRole};

/// Copy is an explicit, complete-body operation, never a display description.
pub fn copy<'a>(group: &[&Event], events: impl Iterator<Item = &'a Event>, parents: &HashMap<String, String>) -> String {
        let mut results: HashMap<&str, Vec<&Event>> = HashMap::new();
        for e in events {
            if e.role == EventRole::Tool && let Some(parent) = parents.get(&e.id) {
                results.entry(parent).or_default().push(e);
            }
        }
        let mut parts = vec![];
        for e in group {
            if e.kind==EventKind::Text && e.role!=EventRole::Tool {parts.push(e.text.clone());continue;}
            if e.kind == EventKind::Thinking && e.role != EventRole::Tool {
                if !e.text.is_empty() {
                    parts.push(format!("Thinking\n{}", e.text));
                }
                continue;
            }
            let mut tool = format!("Tool · {}", e.tool_name.as_deref().unwrap_or("tool"));
            if e.role != EventRole::Tool && !e.text.is_empty() {
                tool.push_str(&format!("\nInput\n{}", e.text));
            }
            let results = results.get(e.id.as_str());
            let orphan = [*e];
            let results = results.map(Vec::as_slice).unwrap_or_else(|| {
                if e.role == EventRole::Tool {
                    &orphan
                } else {
                    &[]
                }
            });
            for result in results {
                if result.kind == EventKind::Text && !result.text.is_empty() {
                    tool.push_str(&format!(
                        "\n{}\n{}",
                        if result.is_error { "Error" } else { "Output" },
                        result.text
                    ));
                }
            }
            parts.push(tool);
        }
        parts.join("\n\n")
}

pub(crate) fn group_state(group: &[&Event], local: &LocalChat) -> (String,bool) {
    let key = group.iter().map(|e|format!("details:{}",e.id)).find(|key|local.expansion.contains_key(key))
        .unwrap_or_else(||format!("details:{}",group[0].id));
    let open = local.expansion.get(&key).copied().unwrap_or_else(||local.details_default || group.iter().any(|e|local.expanded.contains(&e.id)));
    (key,open)
}

/// The same disclosure grouping used by rendering, operating on root headers only.
pub(crate) fn open_items<'a>(events: impl Iterator<Item=&'a Event>, local: &LocalChat) -> HashSet<String> {
    let visible = events.filter(|e| {
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
        if group_state(group,local).1 {open.extend(group.iter().map(|e|e.id.clone()));}
    }
    open
}
