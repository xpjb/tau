use super::*;

impl App {
    pub(super) fn project_tabs(&mut self, layer: &mut Layer, b: Rect) {
        self.root.legacy.projects_rect = b;
        let s = self.ui.scale;
        let projects = &self.controller.account.projects;
        let style = sanscale::Style { chain: self.services.renderer.faces.prose[1], wrap_em: None, align: sanscale::Align::Left, line_spacing: 1. };
        let widths = projects.iter().map(|p| {
            let text = self.services.renderer.text.shape_transient(&p.name, &style).map_or(70. * s, |block| self.services.renderer.text.measure(block).width_em() * 13. * s);
            (text + 24. * s).clamp(56. * s, 220. * s)
        }).collect::<Vec<_>>();
        self.root.legacy.max_project_scroll = (widths.iter().sum::<f32>() + 40. * s - b.width).max(0.);
        let selected_position = projects.iter().position(|p| p.id == self.controller.account.selected_project);
        if self.root.legacy.revealed_project != self.controller.account.selected_project || self.root.legacy.revealed_project_position != selected_position {
            // A selected topic may jump left when a chat inside it is bumped.
            // Keep it in view without undoing intentional scrolling on every frame.
            self.root.legacy.revealed_project = self.controller.account.selected_project.clone();
            self.root.legacy.revealed_project_position = selected_position;
            if let Some(i) = selected_position {
                let left = widths[..i].iter().sum::<f32>();
                let right = left + widths[i];
                if left < self.root.legacy.project_scroll { self.root.legacy.project_scroll = left; }
                else if right > self.root.legacy.project_scroll + b.width { self.root.legacy.project_scroll = right - b.width; }
            }
        }
        self.root.legacy.project_scroll = self.root.legacy.project_scroll.clamp(0., self.root.legacy.max_project_scroll);
        let mut x = b.x + 4. * s - self.root.legacy.project_scroll;
        for (p, w) in projects.iter().zip(widths) {
            let r = Rect::new(x, b.y, w, b.height);
            let hit = crate::render::intersect(r, b);
            if hit.width > 0. {
                let selected = p.id == self.controller.account.selected_project;
                layer.clipped_rounded_rect(r, 4. * s, layer.control_color(r, color(0x0e141b)), b);
                let label = Rect::new(x + 8. * s, b.y + 8. * s, w - 24. * s, 18. * s);
                self.services.renderer.clipped_label(layer, &p.name, label, 13. * s, color(if selected { 0x67d4ff } else { 0xb7c2ce }), selected, crate::render::intersect(label, b));
                if self.controller.project_unread(&p.id) {
                    layer.clipped_rounded_rect(Rect::new(x + w - 12. * s, b.y + 14. * s, 5. * s, 5. * s), 3. * s, color(0x67d4ff), b);
                }
                if selected { layer.clipped_rounded_rect(Rect::new(x + 8. * s, b.y + b.height - 3. * s, w - 16. * s, 3. * s), 1.5 * s, color(0x67d4ff), b); }
                self.root.legacy.project_areas.push((hit, p.id.clone()));
                self.root.legacy.hits.push(Hit { rect: hit, action: Action::SelectProject(p.id.clone()) });
            }
            x += w;
        }
        let add = Rect::new(x, b.y, 36. * s, b.height);
        let hit = crate::render::intersect(add, b);
        if hit.width > 0. {
            layer.clipped_rounded_rect(add, 8. * s, layer.control_color(add, color(0x0e141b)), b);
            self.services.renderer.clipped_label(layer, "+", Rect::new(x + 10. * s, b.y + 3. * s, 24. * s, 28. * s), 22. * s, color(0x67d4ff), false, b);
            self.root.legacy.hits.push(Hit { rect: hit, action: Action::NewProject });
        }
        layer.rect(Rect::new(b.x, b.y + b.height, b.width, s), color(0x2a3541));
    }
    pub(super) fn project_context(&mut self, id: &str, point: Vec2) {
        self.cancel_autoscroll();
        self.root.legacy.usage.dismiss();
        self.root.legacy.info_tip.dismiss();
        self.root.legacy.selecting = false;
        let mut options = vec![];
        if id != GENERAL_PROJECT_ID { options.push(("Rename…".into(), Action::RenameProject(id.into()))); }
        options.push(("Edit topic prompt…".into(), Action::ProjectPrompt(id.into())));
        if id != GENERAL_PROJECT_ID { options.push(("Delete topic…".into(), Action::DeleteProject(id.into()))); }
        self.root.legacy.context_menu = Some(ContextMenu { at: point, section: None, chat: None, options, selected: 0, scroll: 0., parent: None });
        self.root.legacy.pointer = None;
        self.root.legacy.wheel = None;
        self.root.legacy.project_velocity = 0.;
        self.root.legacy.focus = None;
        self.ui.dirty = true;
    }
    pub(super) fn move_menu(&mut self, session: &str) {
        let Some(menu) = self.root.legacy.context_menu.take() else { return; };
        let parent = menu.parent.unwrap_or_else(|| Box::new(ContextMenu { parent: None, ..menu }));
        let current = self.controller.account.sessions.iter().find(|s| s.id == session).map(|s| &s.project_id);
        let mut options = vec![("‹  Move to topic".into(), Action::ContextBack)];
        options.extend(self.controller.account.projects.iter().map(|p| {
            if current == Some(&p.id) { (format!("✓  {}", p.name), Action::Noop) }
            else { (p.name.clone(), Action::MoveChat(session.into(), p.id.clone())) }
        }));
        self.root.legacy.context_menu = Some(ContextMenu { at: parent.at, chat: Some(session.into()), section: None, options, selected: 1, scroll: 0., parent: Some(parent) });
        self.ui.dirty = true;
    }
    pub(super) fn scroll_menu(&mut self, amount: f32) {
        if let Some(menu) = &mut self.root.legacy.context_menu {
            let max = (menu.options.len() as f32 * 36. * self.ui.scale - self.root.legacy.menu_viewport.height).max(0.);
            menu.scroll = (menu.scroll + amount).clamp(0., max);
            self.ui.dirty = true;
        }
    }
    pub(super) fn reveal_menu_selection(&mut self) {
        if let Some(menu) = &mut self.root.legacy.context_menu {
            let top = menu.selected as f32 * 36. * self.ui.scale;
            if top < menu.scroll { menu.scroll = top; }
            else if top + 36. * self.ui.scale > menu.scroll + self.root.legacy.menu_viewport.height { menu.scroll = (top + 36. * self.ui.scale - self.root.legacy.menu_viewport.height).max(0.); }
        }
    }
    pub(super) fn context_frame(&mut self, layer: &mut Layer, bounds: Rect) {
        let Some(menu) = &self.root.legacy.context_menu else { return; };
        let s = self.ui.scale;
        let w = (240. * s).min(bounds.width - 16. * s).max(1.);
        let rect_for = |menu: &ContextMenu, x: f32| {
            let h = ((menu.options.len() as f32 * 36. + 8.) * s).min((bounds.height - 16. * s).max(1.));
            Rect::new(x.clamp(bounds.x + 8. * s, (bounds.x + bounds.width - w - 8. * s).max(bounds.x + 8. * s)),
                menu.at.y.clamp(bounds.y + 8. * s, (bounds.y + bounds.height - h - 8. * s).max(bounds.y + 8. * s)), w, h)
        };
        let mut rect = rect_for(menu, menu.at.x);
        self.root.legacy.hits.clear();
        if let Some(parent) = &menu.parent && bounds.width >= w * 2. + 24. * s {
            let mut parent_rect = rect_for(parent, parent.at.x);
            let x = if parent_rect.x + 2. * w + 8. * s <= bounds.x + bounds.width { parent_rect.x + w }
                else if parent_rect.x - w >= bounds.x { parent_rect.x - w }
                else { parent_rect.x = bounds.x + 8. * s; parent_rect.x + w };
            rect = rect_for(menu, x);
            self.root.legacy.context_rect = Rect::new(rect.x.min(parent_rect.x), rect.y.min(parent_rect.y), w * 2.,
                (rect.y + rect.height).max(parent_rect.y + parent_rect.height) - rect.y.min(parent_rect.y));
            draw_menu(&mut self.services.renderer, layer, &mut self.root.legacy.hits, parent, parent_rect, s, self.root.legacy.hover, false);
        } else { self.root.legacy.context_rect = rect; }
        self.root.legacy.menu_viewport = Rect::new(rect.x + 4. * s, rect.y + 4. * s, rect.width - 8. * s, (rect.height - 8. * s).max(0.));
        draw_menu(&mut self.services.renderer, layer, &mut self.root.legacy.hits, menu, rect, s, self.root.legacy.hover, true);
    }
}
fn draw_menu(renderer: &mut Renderer, layer: &mut Layer, hits: &mut Vec<Hit>, menu: &ContextMenu, rect: Rect, s: f32, hover: Option<Vec2>, active: bool) {
    layer.rounded_rect(rect, 8. * s, color(0x36343b));
    let clip = Rect::new(rect.x + 4. * s, rect.y + 4. * s, rect.width - 8. * s, (rect.height - 8. * s).max(0.));
    let max = (menu.options.len() as f32 * 36. * s - clip.height).max(0.);
    let scroll = menu.scroll.clamp(0., max);
    for (i, (label, action)) in menu.options.iter().enumerate() {
        let r = Rect::new(clip.x, clip.y + i as f32 * 36. * s - scroll, clip.width, 36. * s);
        let hit = crate::render::intersect(r, clip);
        if hit.height <= 0. { continue; }
        if hover.is_some_and(|p| contains(hit, p)) || active && hover.is_none() && menu.selected == i || !active && matches!(action, Action::MoveMenu(_)) {
            layer.clipped_rounded_rect(r, 4. * s, color(0x494750), clip);
        }
        let current = matches!(action, Action::Noop);
        renderer.clipped_label(layer, label, Rect::new(r.x + 12. * s, r.y + 9. * s, r.width - 24. * s, 22. * s), 14. * s,
            color(if current { 0x82909f } else if matches!(action, Action::DeleteProject(_) | Action::Delete(_)) { 0xffb4ab } else { 0xe5eaf0 }), false, clip);
        hits.push(Hit { rect: hit, action: action.clone() });
    }
    if max > 0. {
        let h = (clip.height * clip.height / (max + clip.height)).max(18. * s).min(clip.height);
        layer.rounded_rect(Rect::new(rect.x + rect.width - 4. * s, clip.y + (clip.height - h) * scroll / max, 2. * s, h), s, color(0x82909f));
    }
}
