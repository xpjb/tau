//! Paths are stored only for active targets, not as a parallel widget tree.
//! Concrete parents validate their children's lifetime/visibility before the
//! next event; capture can therefore never survive an ancestor being hidden.
use super::*;
impl RootWidget {
    pub fn active_route(&self, target: Target, ui: &UiState) -> Option<Vec<Id>> {
        let scope = target.scope;
        if let Some(dialog) = &self.dialog {
            return (scope == dialog.id()).then(|| vec![self.id, dialog.id()]);
        }
        if let Some(viewer) = &self.viewer {
            return (scope == viewer.id).then(|| vec![self.id, viewer.id]);
        }
        if let Some(menu) = &self.menu {
            let mut path = vec![self.id, menu.id];
            let mut current = menu.as_ref();
            loop {
                if scope == current.id {
                    return Some(path);
                }
                let parent = current.parent.as_ref()?;
                path.push(parent.id);
                current = parent;
            }
        }
        if scope == self.notice.body.target.scope {
            return Some(vec![self.id, scope]);
        }
        #[cfg(test)]
        if self.test_cards.cards.values().any(|c| c.controls.id == scope) {
            return Some(vec![self.id, scope]);
        }
        let workspace = &self.workspace;
        let mut path = vec![self.id, workspace.id];
        let width = ui.size.0 as f32 / ui.scale;
        let screen = workspace.attachments.show && (ui.mobile || width < 1000.);
        if workspace.attachments.show {
            let owner = workspace.attachments.scroll.target.scope;
            if scope == owner {
                return Some(vec![self.id, workspace.id, owner]);
            }
            if workspace.attachments.cards.cards.values().any(|c| c.controls.id == scope) {
                return Some(vec![self.id, workspace.id, owner, scope]);
            }
        }
        if screen {
            return None;
        }
        if width >= 760. || workspace.show_chats {
            let sidebar = &workspace.sidebar;
            if scope == sidebar.controls.id {
                return Some(vec![self.id, workspace.id, scope]);
            }
            if scope == sidebar.projects.controls.id {
                return Some(vec![self.id, workspace.id, sidebar.controls.id, scope]);
            }
        }
        if width < 760. && workspace.show_chats {
            return None;
        }
        let chat = &workspace.chat;
        path.push(chat.id);
        if let Some(view) = &chat.code.view {
            if scope == chat.code.controls.id {
                path.push(scope);
                return Some(path);
            }
            if scope == view.id {
                path.extend([chat.code.controls.id, view.id]);
                return Some(path);
            }
            if view.search.is_some() || view.document.is_none() {
                return None;
            }
        } else {
            if scope == chat.header.controls.id {
                path.push(scope);
                return Some(path);
            }
            let transcript = &chat.transcript;
            let owner = transcript.scroll.target.scope;
            if scope == owner {
                path.push(owner);
                return Some(path);
            }
            if scope == transcript.models.controls.id {
                path.extend([owner, scope]);
                return Some(path);
            }
            for row in &transcript.rows {
                let row_id = row.control.target.scope;
                if scope == row_id {
                    path.extend([owner, row_id]);
                    return Some(path);
                }
                if row.attachment.as_ref().is_some_and(|card| card.controls.id == scope) {
                    path.extend([owner, row_id, scope]);
                    return Some(path);
                }
            }
        }
        if scope == chat.composer.controls.id {
            path.push(scope);
            return Some(path);
        }
        None
    }
}
