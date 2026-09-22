//! Tau 1's disclosure hierarchy: Details → tool → large Input/Output sections.
//! Tool results are paired by call ID, including results whose call is not in the loaded page.
use crate::store::LocalChat;
use std::collections::{HashMap, HashSet};
use tau_protocol::{Event, EventKind, EventPhase, EventRole};

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
        Self { calls, results }
    }
    pub fn paired_result(&self, e: &Event) -> bool {
        e.role == EventRole::Tool
            && e.tool_call_id.as_deref().is_some_and(|id| {
                self.calls.contains(id)
                    || self.results[id]
                        .first()
                        .is_some_and(|first| first.id != e.id)
            })
    }
    pub fn lines(&self, group: &[&Event], local: &LocalChat) -> Vec<Line> {
        // Reuse an explicitly toggled group key when a previous page prepends more details.
        let key = group
            .iter()
            .map(|e| format!("details:{}", e.id))
            .find(|key| local.expansion.contains_key(key))
            .unwrap_or_else(|| format!("details:{}", group[0].id));
        let open = local.expansion.get(&key).copied().unwrap_or_else(|| {
            local.details_default || group.iter().any(|e| local.expanded.contains(&e.id))
        });
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
                        source: if e.phase == EventPhase::Saved {
                            e.text.clone()
                        } else {
                            crate::app::literal(&e.text)
                        },
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
            let error = results.last().is_some_and(|e| e.is_error);
            let open = local.expansion.get(&key).copied().unwrap_or(false);
            lines.push(Line::label(
                key.clone(),
                format!("Tool · {}", e.tool_name.as_deref().unwrap_or("tool")),
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
            for (label, text) in [
                ("Input", input),
                (if error { "Error" } else { "Output" }, output.as_str()),
            ] {
                if text.is_empty() {
                    continue;
                }
                let section = format!("{key}:{label}");
                let large = text.chars().count() > 1200
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
                        source: crate::app::code(text),
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
