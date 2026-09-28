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
        self.root.tooltips.usage.dismiss();
        self.root.tooltips.info.dismiss();
        self.root.legacy.selecting = false;
        let mut options = vec![];
        if id != GENERAL_PROJECT_ID { options.push(("Rename…".into(), ui::MenuChoice::RenameProject(id.into()))); }
        options.push(("Edit topic prompt…".into(), ui::MenuChoice::ProjectPrompt(id.into())));
        if id != GENERAL_PROJECT_ID { options.push(("Delete topic…".into(), ui::MenuChoice::DeleteProject(id.into()))); }
        self.with_ui(|_, cx| {
            let menu = ui::Menu::new(point, None, None, options, cx);
            cx.ui.requests.push_back(ui::Request::Menu(Box::new(menu)));
        });
        if let Err(error) = self.finish_ui_requests() { self.report(Err(error)); }
        self.root.legacy.pointer = None;
        self.root.legacy.wheel = None;
        self.root.legacy.project_velocity = 0.;
        self.root.legacy.focus = None;
        self.ui.dirty = true;
    }
}
