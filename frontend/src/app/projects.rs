use super::*;

pub(super) fn is_project_modal(kind: &ModalKind) -> bool {
    matches!(kind, ModalKind::NewProject(_) | ModalKind::RenameProject(_) | ModalKind::ProjectPrompt(_) | ModalKind::DeleteProject(_) | ModalKind::DeleteProjectChoice(_))
}
impl App {
    fn project(&self, id: &str) -> Result<Project> {
        self.controller.account.projects.iter().find(|p| p.id == id).cloned()
            .ok_or_else(|| anyhow::anyhow!("Topic no longer exists"))
    }
    pub(super) fn project_action(&mut self, action: Action) -> Result<()> {
        if self.saving_project.is_some() { return Ok(()); }
        self.controller.notice = None;
        let options = vec![("Save".into(), Action::Confirm), ("Cancel".into(), Action::CancelModal)];
        self.modal = Some(match action {
            Action::NewProject => Modal {
                kind: ModalKind::NewProject(uuid::Uuid::new_v4().to_string()), title: "New topic".into(),
                fields: vec![("Name".into(), Editor::line(String::new()), false), ("Topic prompt (optional)".into(), Editor::new(String::new()), false)], options,
            },
            Action::RenameProject(id) => {
                let p = self.project(&id)?;
                anyhow::ensure!(id != GENERAL_PROJECT_ID, "General cannot be renamed");
                Modal { title: "Rename topic".into(), fields: vec![("Name".into(), Editor::line(p.name.clone()), false)], kind: ModalKind::RenameProject(p), options }
            }
            Action::ProjectPrompt(id) => {
                let p = self.project(&id)?;
                Modal { title: format!("{} — topic prompt", p.name), fields: vec![("Topic prompt".into(), Editor::new(p.prompt.clone()), false)], kind: ModalKind::ProjectPrompt(p), options }
            }
            Action::DeleteProject(id) => {
                let p = self.project(&id)?;
                anyhow::ensure!(id != GENERAL_PROJECT_ID, "General cannot be deleted");
                Modal { title: format!("Delete topic “{}”?", p.name), fields: vec![], kind: ModalKind::DeleteProject(p),
                    options: vec![("Continue…".into(), Action::Confirm), ("Cancel".into(), Action::CancelModal)] }
            }
            Action::RemoveProject(mode) => {
                let Some(Modal { kind: ModalKind::DeleteProjectChoice(p), .. }) = &self.modal else { return Ok(()); };
                let command = ClientCommand::DeleteProject { project_id: p.id.clone(), revision: p.revision, mode };
                self.send_project(command)?;
                return Ok(());
            }
            _ => return Ok(()),
        });
        self.focus = self.modal.as_ref().filter(|m| !m.fields.is_empty()).map(|_| Some(0));
        Ok(())
    }
    fn send_project(&mut self, command: ClientCommand) -> Result<()> {
        self.controller.project_result = None;
        self.saving_project = Some(self.controller.request(command)?);
        self.focus = None;
        Ok(())
    }
    pub(super) fn confirm_project(&mut self) -> Result<()> {
        if self.saving_project.is_some() { return Ok(()); }
        let Some(modal) = &self.modal else { return Ok(()); };
        let value = |i: usize| modal.fields[i].1.value.clone();
        let command = match &modal.kind {
            ModalKind::NewProject(id) => ClientCommand::CreateProject { project_id: id.clone(), name: value(0), prompt: value(1) },
            ModalKind::RenameProject(p) => ClientCommand::UpdateProject { project_id: p.id.clone(), revision: p.revision, name: value(0), prompt: p.prompt.clone() },
            ModalKind::ProjectPrompt(p) => ClientCommand::UpdateProject { project_id: p.id.clone(), revision: p.revision, name: p.name.clone(), prompt: value(0) },
            ModalKind::DeleteProject(p) => {
                let p = p.clone();
                self.modal = Some(Modal { title: format!("Delete “{}” — what happens to its chats?", p.name), fields: vec![], kind: ModalKind::DeleteProjectChoice(p),
                    options: vec![("Move chats to General".into(), Action::RemoveProject(DeleteProjectMode::MoveToGeneral)),
                        ("Delete topic and its chats".into(), Action::RemoveProject(DeleteProjectMode::DeleteChats)), ("Cancel".into(), Action::CancelModal)] });
                self.focus = None;
                return Ok(());
            }
            _ => return Ok(()),
        };
        if let ClientCommand::CreateProject { name, prompt, .. } | ClientCommand::UpdateProject { name, prompt, .. } = &command {
            anyhow::ensure!(!name.trim().is_empty() && name.chars().count() <= MAX_PROJECT_NAME_CHARS && !name.chars().any(char::is_control), "Use a topic name of 1–{MAX_PROJECT_NAME_CHARS} characters on one line");
            anyhow::ensure!(prompt.chars().count() <= MAX_PROJECT_PROMPT_CHARS, "Topic prompt is too long");
        }
        self.send_project(command)
    }
    pub(super) fn project_result(&mut self) {
        let Some(request) = &self.saving_project else { return; };
        if self.controller.project_result.as_ref().is_some_and(|(id,_)| id == request) {
            let ok = self.controller.project_result.take().unwrap().1;
            self.saving_project = None;
            if ok {
                self.modal = None;
                self.focus = None;
                if self.controller.account.selected.is_none() { self.show_chats = true; }
            }
            self.dirty = true;
        } else if self.controller.epoch.is_none() {
            self.saving_project = None;
            self.controller.notice = Some("Topic change unconfirmed. Reconnect and check before trying again; it was not resent.".into());
            self.dirty = true;
        }
    }
    pub(super) fn project_tabs(&mut self, layer: &mut Layer, b: Rect) {
        self.projects_rect = b;
        let s = self.scale;
        let projects = &self.controller.account.projects;
        let style = sanscale::Style { chain: self.renderer.faces.prose[1], wrap_em: None, align: sanscale::Align::Left, line_spacing: 1. };
        let widths = projects.iter().map(|p| {
            let text = self.renderer.text.shape_transient(&p.name, &style).map_or(70. * s, |block| self.renderer.text.measure(block).width_em() * 13. * s);
            (text + 24. * s).clamp(56. * s, 220. * s)
        }).collect::<Vec<_>>();
        self.max_project_scroll = (widths.iter().sum::<f32>() + 40. * s - b.width).max(0.);
        if self.revealed_project != self.controller.account.selected_project {
            self.revealed_project = self.controller.account.selected_project.clone();
            if let Some(i) = projects.iter().position(|p| p.id == self.revealed_project) {
                let left = widths[..i].iter().sum::<f32>();
                let right = left + widths[i];
                if left < self.project_scroll { self.project_scroll = left; }
                else if right > self.project_scroll + b.width { self.project_scroll = right - b.width; }
            }
        }
        self.project_scroll = self.project_scroll.clamp(0., self.max_project_scroll);
        let mut x = b.x + 4. * s - self.project_scroll;
        for (p, w) in projects.iter().zip(widths) {
            let r = Rect::new(x, b.y, w, b.height);
            let hit = crate::render::intersect(r, b);
            if hit.width > 0. {
                let selected = p.id == self.controller.account.selected_project;
                layer.clipped_rounded_rect(r, 4. * s, layer.control_color(r, color(0x0e141b)), b);
                let label = Rect::new(x + 8. * s, b.y + 8. * s, w - 24. * s, 18. * s);
                self.renderer.clipped_label(layer, &p.name, label, 13. * s, color(if selected { 0x67d4ff } else { 0xb7c2ce }), selected, crate::render::intersect(label, b));
                if self.controller.project_unread(&p.id) {
                    layer.clipped_rounded_rect(Rect::new(x + w - 12. * s, b.y + 14. * s, 5. * s, 5. * s), 3. * s, color(0x67d4ff), b);
                }
                if selected { layer.clipped_rounded_rect(Rect::new(x + 8. * s, b.y + b.height - 3. * s, w - 16. * s, 3. * s), 1.5 * s, color(0x67d4ff), b); }
                self.project_areas.push((hit, p.id.clone()));
                self.hits.push(Hit { rect: hit, action: Action::SelectProject(p.id.clone()) });
            }
            x += w;
        }
        let add = Rect::new(x, b.y, 36. * s, b.height);
        let hit = crate::render::intersect(add, b);
        if hit.width > 0. {
            layer.clipped_rounded_rect(add, 8. * s, layer.control_color(add, color(0x0e141b)), b);
            self.renderer.clipped_label(layer, "+", Rect::new(x + 10. * s, b.y + 3. * s, 24. * s, 28. * s), 22. * s, color(0x67d4ff), false, b);
            self.hits.push(Hit { rect: hit, action: Action::NewProject });
        }
        layer.rect(Rect::new(b.x, b.y + b.height, b.width, s), color(0x2a3541));
    }
    pub(super) fn project_context(&mut self, id: &str, point: Vec2) {
        self.cancel_autoscroll();
        self.usage.dismiss();
        self.info_tip.dismiss();
        self.selecting = false;
        let mut options = vec![];
        if id != GENERAL_PROJECT_ID { options.push(("Rename…".into(), Action::RenameProject(id.into()))); }
        options.push(("Edit topic prompt…".into(), Action::ProjectPrompt(id.into())));
        if id != GENERAL_PROJECT_ID { options.push(("Delete topic…".into(), Action::DeleteProject(id.into()))); }
        self.context_menu = Some(ContextMenu { at: point, section: None, chat: None, options, selected: 0, scroll: 0., parent: None });
        self.pointer = None;
        self.wheel = None;
        self.project_velocity = 0.;
        self.focus = None;
        self.dirty = true;
    }
    pub(super) fn move_menu(&mut self, session: &str) {
        let Some(menu) = self.context_menu.take() else { return; };
        let parent = menu.parent.unwrap_or_else(|| Box::new(ContextMenu { parent: None, ..menu }));
        let current = self.controller.account.sessions.iter().find(|s| s.id == session).map(|s| &s.project_id);
        let mut options = vec![("‹  Move to topic".into(), Action::ContextBack)];
        options.extend(self.controller.account.projects.iter().map(|p| {
            if current == Some(&p.id) { (format!("✓  {}", p.name), Action::Noop) }
            else { (p.name.clone(), Action::MoveChat(session.into(), p.id.clone())) }
        }));
        self.context_menu = Some(ContextMenu { at: parent.at, chat: Some(session.into()), section: None, options, selected: 1, scroll: 0., parent: Some(parent) });
        self.dirty = true;
    }
    pub(super) fn scroll_menu(&mut self, amount: f32) {
        if let Some(menu) = &mut self.context_menu {
            let max = (menu.options.len() as f32 * 36. * self.scale - self.menu_viewport.height).max(0.);
            menu.scroll = (menu.scroll + amount).clamp(0., max);
            self.dirty = true;
        }
    }
    pub(super) fn reveal_menu_selection(&mut self) {
        if let Some(menu) = &mut self.context_menu {
            let top = menu.selected as f32 * 36. * self.scale;
            if top < menu.scroll { menu.scroll = top; }
            else if top + 36. * self.scale > menu.scroll + self.menu_viewport.height { menu.scroll = (top + 36. * self.scale - self.menu_viewport.height).max(0.); }
        }
    }
    pub(super) fn context_frame(&mut self, layer: &mut Layer, bounds: Rect) {
        let Some(menu) = &self.context_menu else { return; };
        let s = self.scale;
        let w = (240. * s).min(bounds.width - 16. * s).max(1.);
        let rect_for = |menu: &ContextMenu, x: f32| {
            let h = ((menu.options.len() as f32 * 36. + 8.) * s).min((bounds.height - 16. * s).max(1.));
            Rect::new(x.clamp(bounds.x + 8. * s, (bounds.x + bounds.width - w - 8. * s).max(bounds.x + 8. * s)),
                menu.at.y.clamp(bounds.y + 8. * s, (bounds.y + bounds.height - h - 8. * s).max(bounds.y + 8. * s)), w, h)
        };
        let mut rect = rect_for(menu, menu.at.x);
        self.hits.clear();
        if let Some(parent) = &menu.parent && bounds.width >= w * 2. + 24. * s {
            let mut parent_rect = rect_for(parent, parent.at.x);
            let x = if parent_rect.x + 2. * w + 8. * s <= bounds.x + bounds.width { parent_rect.x + w }
                else if parent_rect.x - w >= bounds.x { parent_rect.x - w }
                else { parent_rect.x = bounds.x + 8. * s; parent_rect.x + w };
            rect = rect_for(menu, x);
            self.context_rect = Rect::new(rect.x.min(parent_rect.x), rect.y.min(parent_rect.y), w * 2.,
                (rect.y + rect.height).max(parent_rect.y + parent_rect.height) - rect.y.min(parent_rect.y));
            draw_menu(&mut self.renderer, layer, &mut self.hits, parent, parent_rect, s, self.hover, false);
        } else { self.context_rect = rect; }
        self.menu_viewport = Rect::new(rect.x + 4. * s, rect.y + 4. * s, rect.width - 8. * s, (rect.height - 8. * s).max(0.));
        draw_menu(&mut self.renderer, layer, &mut self.hits, menu, rect, s, self.hover, true);
    }
    pub(super) fn project_modal_frame(&mut self, layer: &mut Layer, b: Rect) {
        let s = self.scale;
        let busy = self.saving_project.is_some();
        self.hits.clear();
        layer.rect(b, sanscale::Color([0.,0.,0.,0.8]));
        let modal = self.modal.as_mut().unwrap();
        let prompt = matches!(modal.kind, ModalKind::NewProject(_) | ModalKind::ProjectPrompt(_));
        let width = (b.width - 24. * s).min(620. * s).max(1.);
        let height = ((if prompt { 550. } else { 340. }) * s).min((b.height - 24. * s).max(1.));
        let r = Rect::new(b.x + (b.width - width) / 2., b.y + (b.height - height) / 2., width, height);
        layer.rounded_rect(r, 16. * s, color(0x111b25));
        let x = r.x + 18. * s;
        let w = (r.width - 36. * s).max(1.);
        self.renderer.label(layer, &modal.title, Rect::new(x, r.y + 16. * s, w, 48. * s), 17. * s, color(0xe5eaf0), true);
        let help = if prompt { "New chats capture this prompt. Edits do not change existing chats. Moving a chat here replaces its topic prompt." }
            else if matches!(modal.kind, ModalKind::DeleteProjectChoice(_)) { "Moving keeps the chats and their history. Deleting chats permanently removes them and cannot be undone." }
            else if matches!(modal.kind, ModalKind::DeleteProject(_)) { "Next, choose whether to keep its chats in General or permanently delete them." } else { "Changes appear on all connected devices." };
        let help_h = if height / s < 400. && prompt { 0. } else { 52. * s };
        if help_h > 0. { self.renderer.label(layer, help, Rect::new(x, r.y + 65. * s, w, help_h), 12. * s, color(0xb7c2ce), false); }
        let footer = r.y + r.height - 14. * s - modal.options.len() as f32 * 42. * s;
        let notice_h = if self.controller.notice.is_some() || busy { 42. * s } else { 0. };
        let mut y = r.y + 70. * s + help_h;
        let field_count = modal.fields.len();
        for (i, (label, editor, secret)) in modal.fields.iter_mut().enumerate() {
            self.renderer.label(layer, label, Rect::new(x, y, w, 18. * s), 11. * s, color(0xb7c2ce), false);
            y += 20. * s;
            let available = (footer - notice_h - 8. * s - y).max(1.);
            let h = if editor.single_line { (40. * s).min((available - (field_count - i - 1) as f32 * 48. * s).max(1.)) } else { available };
            let field = Rect::new(x, y, w, h);
            editor.draw(&mut self.renderer, layer, field, 15. * s, !busy && self.focus == Some(Some(i)), *secret, "", true);
            if !busy { self.hits.push(Hit { rect: field, action: Action::Focus(Some(i)) }); }
            y += h + 10. * s;
        }
        if notice_h > 0. { self.renderer.label(layer, self.controller.notice.as_deref().unwrap_or("Saving…"), Rect::new(x, footer - notice_h, w, notice_h - 4. * s), 12. * s, color(0xffb4ab), false); }
        for (i, (label, action)) in modal.options.iter().enumerate() {
            if busy && !matches!(action, Action::CancelModal) { continue; }
            button(&mut self.renderer, layer, &mut self.hits, Rect::new(x, footer + i as f32 * 42. * s, w, 36. * s), label, action.clone(), s, matches!(action, Action::Confirm));
        }
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
