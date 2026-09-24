//! Tau 1's disclosure hierarchy: Details → tool → large Input/Output sections.
//! Tool results are paired by call ID, including results whose call is not in the loaded page.
use crate::store::LocalChat;
use std::collections::{HashMap, HashSet};
use tau_protocol::{Event, EventKind, EventRole};

#[derive(Clone)]
pub struct Line {
    pub key: String,
    pub label: String,
    pub source: String,
    pub indent: f32,
    pub toggle: Option<bool>,
    pub code: bool,
    pub error: bool,
    pub tool: bool,
}
impl Line {
    fn label(key: String, label: String, indent: f32, toggle: Option<bool>, error: bool) -> Self {
        Self {
            key,
            label,
            indent,
            toggle,
            error,
            source: String::new(),
            code: false,
            tool: indent > 0.,
        }
    }
}
pub struct Tools<'a> {
    calls: HashSet<&'a str>,
    results: HashMap<&'a str, Vec<&'a Event>>,
    lengths: Option<&'a HashMap<String,u64>>,
    states: Option<&'a HashMap<String,String>>,
}
impl<'a> Tools<'a> {
    pub fn new(events: impl Iterator<Item = &'a Event>) -> Self {
        let mut calls = HashSet::new();
        let mut results: HashMap<&str, Vec<&Event>> = HashMap::new();
        for e in events {
            if let Some(id) = e.tool_call_id.as_deref() {
                if e.role == EventRole::Tool {
                    results.entry(id).or_default().push(e);
                } else if e.kind == EventKind::Tool {
                    calls.insert(id);
                }
            }
        }
        Self { calls, results, lengths:None, states:None }
    }
    pub fn with_lengths(mut self, lengths: &'a HashMap<String,u64>) -> Self { self.lengths = Some(lengths); self }
    pub fn with_states(mut self, states:&'a HashMap<String,String>) -> Self {self.states=Some(states);self}
    fn length(&self, event: &Event) -> u64 { self.lengths.and_then(|lengths|lengths.get(&event.id)).copied().unwrap_or(event.text.len() as u64) }
    pub fn paired_result(&self, e: &Event) -> bool {
        e.role == EventRole::Tool
            && e.tool_call_id.as_deref().is_some_and(|id| {
                self.calls.contains(id)
                    || self.results[id]
                        .first()
                        .is_some_and(|first| first.id != e.id)
            })
    }
    /// Copy this logical Details section, including currently collapsed tools,
    /// without pulling in the adjacent assistant answer. Only assembled on demand.
    pub fn copy(&self, group: &[&Event]) -> String {
        let mut parts = vec![];
        for e in group {
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
            let results = e
                .tool_call_id
                .as_deref()
                .and_then(|id| self.results.get(id));
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
    pub fn lines(&self, group: &[&Event], local: &LocalChat) -> Vec<Line> {
        // Reuse an explicitly toggled group key when a previous page prepends more details.
        let (key,open) = group_state(group,local);
        let mut lines = vec![Line::label(key, "Details".into(), 0., Some(open), false)];
        if !open {
            return lines;
        }
        for e in group {
            if e.kind == EventKind::Thinking && e.role != EventRole::Tool {
                if !e.text.is_empty() {
                    lines.push(Line {
                        key: format!("thinking:{}", e.id),
                        label: String::new(),
                        // Live prefixes are Markdown too; completed syntax can
                        // render before the event is saved or the line ends.
                        source: e.text.clone(),
                        indent: 0.,
                        toggle: None,
                        code: false,
                        error: false,
                        tool: false,
                    });
                }
                continue;
            }
            let key = format!("tool:{}", e.tool_call_id.as_deref().unwrap_or(&e.id));
            let results = e
                .tool_call_id
                .as_deref()
                .and_then(|id| self.results.get(id))
                .cloned()
                .unwrap_or_else(|| {
                    if e.role == EventRole::Tool {
                        vec![*e]
                    } else {
                        vec![]
                    }
                });
            let error = e.is_error || results.last().is_some_and(|e| e.is_error);
            let open = local.expansion.get(&key).copied().unwrap_or(false);
            lines.push(Line::label(
                key.clone(),
                format!("Tool · {}{}", e.tool_name.as_deref().unwrap_or("tool"),
                    self.states.and_then(|states|states.get(&e.id)).map(|state|format!(" · {state}")).unwrap_or_default()),
                8.,
                Some(open),
                error,
            ));
            if !open {
                continue;
            }
            let input = if e.role == EventRole::Tool {
                ""
            } else {
                &e.text
            };
            let output = results
                .iter()
                .filter(|e| e.kind == EventKind::Text)
                .map(|e| e.text.as_str())
                .collect::<Vec<_>>()
                .join("\n\n");
            let input_length = if e.role == EventRole::Tool {0} else {self.length(e)};
            let output_length = results.iter().filter(|e|e.kind == EventKind::Text).map(|e|self.length(e)).sum::<u64>();
            for (label, text, length) in [
                ("Input", input, input_length),
                (if error { "Error" } else { "Output" }, output.as_str(), output_length),
            ] {
                if text.is_empty() && length == 0 {
                    continue;
                }
                let section = format!("{key}:{label}");
                let large = length > 1200
                    || text.bytes().filter(|b| *b == b'\n').count() >= 16;
                let open = !large || local.expansion.get(&section).copied().unwrap_or(false);
                lines.push(Line::label(
                    section.clone(),
                    label.into(),
                    16.,
                    large.then_some(open),
                    label == "Error",
                ));
                if open {
                    lines.push(Line {
                        key: format!("{section}:text"),
                        label: String::new(),
                        source: if text.is_empty() { "Loading…".into() } else {crate::app::code(text)},
                        indent: 16.,
                        toggle: None,
                        code: true,
                        error: false,
                        tool: true,
                    });
                }
            }
        }
        lines
    }
}

fn group_state(group: &[&Event], local: &LocalChat) -> (String,bool) {
    let key = group.iter().map(|e|format!("details:{}",e.id)).find(|key|local.expansion.contains_key(key))
        .unwrap_or_else(||format!("details:{}",group[0].id));
    let open = local.expansion.get(&key).copied().unwrap_or_else(||local.details_default || group.iter().any(|e|local.expanded.contains(&e.id)));
    (key,open)
}

/// The same disclosure grouping used by rendering, operating on headers alone.
pub(crate) fn open_items<'a>(events: impl Iterator<Item=&'a Event>, local: &LocalChat) -> HashSet<String> {
    let events = events.collect::<Vec<_>>();
    let tools = Tools::new(events.iter().copied());
    let visible = events.into_iter().filter(|e| {
        let empty = e.attachment.is_none() && e.error_message.is_none() && !e.is_error &&
            (e.kind == EventKind::Hidden || matches!(e.kind,EventKind::Thinking|EventKind::Text) && e.text.is_empty());
        !empty && !(e.attachment.is_none() && tools.paired_result(e))
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
