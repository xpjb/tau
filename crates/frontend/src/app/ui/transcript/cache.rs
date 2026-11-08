//! Scroll geometry is cheap to retain; interaction owners are never cached.
//! Keep recently used chats until the row/entry budget needs their space.
use super::Placed;
use std::collections::VecDeque;

const MAX_ROWS: usize = 32_768;
const MAX_CHATS: usize = 128;

pub(super) struct Layout {
    pub measured: (u64, u32, u32),
    pub placed: Vec<Placed>,
}
#[derive(Default)]
pub(super) struct Cache {
    entries: VecDeque<(String, Layout)>,
    rows: usize,
}
impl Cache {
    pub fn take(&mut self, session: &str) -> Option<Layout> {
        let index = self.entries.iter().position(|(id, _)| id == session)?;
        let (_, layout) = self.entries.remove(index).unwrap();
        self.rows -= layout.placed.len();
        Some(layout)
    }
    pub fn put(&mut self, session: String, layout: Layout) {
        self.take(&session);
        self.rows += layout.placed.len();
        self.entries.push_back((session, layout));
    }
    pub fn trim(&mut self, active_rows: usize) {
        while self.rows.saturating_add(active_rows) > MAX_ROWS || self.entries.len() > MAX_CHATS {
            let Some((_, layout)) = self.entries.pop_front() else { break; };
            self.rows -= layout.placed.len();
        }
    }
}

#[cfg(test)]
#[path = "../../../../tests/unit/app/ui/transcript_cache.rs"]
mod tests;
