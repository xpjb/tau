//! Sibling ownership for the retained UI migration. The legacy workspace is a
//! temporary adapter, not the owner of the model, renderer or shared UI state.
use super::{LegacyWorkspace, PlatformAction, Renderer, Vec2};

pub(super) struct RootWidget {
    pub(super) legacy: LegacyWorkspace,
}

pub(super) struct Services {
    pub(super) renderer: Renderer,
    pub(super) platform: Vec<PlatformAction>,
}

pub(crate) struct UiState {
    pub(crate) size: (u32, u32),
    pub(crate) origin: Vec2,
    pub(crate) scale: f32,
    pub(crate) mobile: bool,
    pub(crate) window_focused: bool,
    pub(crate) dirty: bool,
}
