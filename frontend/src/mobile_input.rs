//! The Android IME edits the same field that Rust renders, never another screen.
//! Editor IDs and revisions fence delayed callbacks after Send, navigation or a
//! Rust-side caret move. Android owns composition between those revisions.
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Input {
    pub id: u64,
    pub revision: u64,
    pub request: u64,
    pub text: String,
    pub start: i32,
    pub end: i32,
    pub secret: bool,
    pub single_line: bool,
    pub rect: [f32; 4],
    pub size: f32,
    pub max_bytes: usize,
}
impl Input {
    pub fn same_configuration(&self, other: &Self) -> bool {
        // Native edits already live in the IME. Never echo an older snapshot
        // back into its Editable (which would destroy composing spans/cursor).
        self.id == other.id && self.revision == other.revision
            && self.request == other.request && self.rect == other.rect
            && self.size == other.size && self.secret == other.secret
            && self.single_line == other.single_line
    }
}

pub struct Edit {
    pub id: u64,
    pub revision: u64,
    pub text: String,
    pub start: i32,
    pub end: i32,
    pub composing_start: i32,
    pub composing_end: i32,
}

/// All rectangles are absolute surface coordinates, not bottom insets. Taking
/// their intersection works both with a resized surface and edge-to-edge: an
/// IME already excluded by Android's content rect must not be subtracted twice.
pub fn viewport(size: (u32, u32), native: [i32; 4], visible: [i32; 4]) -> [u32; 4] {
    let mut bounds = [0, 0, size.0, size.1];
    for rect in [native, visible] {
        if rect[2] > rect[0] && rect[3] > rect[1] {
            bounds[0] = bounds[0].max(rect[0].max(0) as u32);
            bounds[1] = bounds[1].max(rect[1].max(0) as u32);
            bounds[2] = bounds[2].min(rect[2].max(0) as u32);
            bounds[3] = bounds[3].min(rect[3].max(0) as u32);
        }
    }
    bounds
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ime_viewport_is_not_subtracted_twice_and_restores_after_hide() {
        let visible = [0, 24, 360, 420];
        assert_eq!(viewport((360, 800), [0, 24, 360, 800], visible), [0, 24, 360, 420]);
        assert_eq!(viewport((360, 420), [0, 24, 360, 420], visible), [0, 24, 360, 420]);
        assert_eq!(viewport((360, 800), [0, 24, 360, 776], [0, 24, 360, 776]), [0, 24, 360, 776]);
        assert_eq!(viewport((800, 360), [0; 4], [30, 0, 776, 180]), [30, 0, 776, 180]);
    }
}
