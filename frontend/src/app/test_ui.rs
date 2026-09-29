//! Test-only setup and selectors over real placed controls, never runtime dispatch.
use super::*;
#[derive(Clone)]
pub(super) enum FixtureChoice {
    Select(String),
    New,
    SelectProject(String),
    NewProject,
    RenameProject(String),
    ProjectPrompt(String),
    DeleteProject(String),
    RetryCreate,
    Settings,
    ModelSettings,
    ChooseModel(String, String),
    Usage,
    Info(Info),
    Back,
    Send,
    Abort,
    Tail,
    DismissNotice,
    OpenDownloadNotice(DownloadTarget),
    ReviewRestore(String),
    Outbox(usize),
    InspectControl(String),
    ForgetControl(String),
    Composer,
    CodeSearch,
    Confirm,
    CancelModal,
    AgentSetting(String, String),
    Delete(String),
    Attach,
    Attachments,
    Files,
    FileOpen(String, bool),
    FileUp,
    FileClose,
    FileChat,
    FileHidden,
    FileSelect(String),
    FileAccept,
    FileFind,
    FileFindHere,
    FileClear,
    FileCopy,
    FilePage(bool),
    RemoveFile(String),
    Toggle(String, bool),
    Queue(QueueOperation),
    Suggest(String),
}

pub(super) struct PlacedControl {
    pub rect: Rect,
    pub action: FixtureChoice,
}

impl App {
    pub(super) fn placed_controls(&self) -> Vec<PlacedControl> {
        let mut hits = vec![];
        for (_, button, choice) in &self.root.workspace.sidebar.controls.items {
            use ui::sidebar::Choice;
            if let Some(r) = button.control.rect
                && button.control.enabled
            {
                let rect = crate::render::intersect(r, button.control.clip);
                if rect.width <= 0. || rect.height <= 0. {
                    continue;
                }
                let action = match choice {
                    Choice::Select(id) => FixtureChoice::Select(id.clone()),
                    Choice::New => FixtureChoice::New,
                    Choice::Settings => FixtureChoice::Settings,
                    Choice::Info(i) => FixtureChoice::Info(i.clone()),
                };
                hits.push(PlacedControl { rect, action });
            }
        }
        for (_, button, choice) in &self.root.workspace.sidebar.projects.controls.items {
            if let Some(r) = button.control.rect {
                let rect = crate::render::intersect(r, button.control.clip);
                if rect.width <= 0. || rect.height <= 0. {
                    continue;
                }
                hits.push(PlacedControl {
                    rect,
                    action: match choice {
                        ui::sidebar::TopicChoice::Select(id) => FixtureChoice::SelectProject(id.clone()),
                        ui::sidebar::TopicChoice::New => FixtureChoice::NewProject,
                    },
                });
            }
        }
        if let Some(rect) = self.root.notice.body.rect {
            hits.push(PlacedControl {
                rect,
                action: self
                    .controller
                    .notice
                    .as_ref()
                    .and_then(|n| n.download.clone())
                    .map_or(FixtureChoice::DismissNotice, FixtureChoice::OpenDownloadNotice),
            });
        }
        if let Some(rect) = self.root.notice.close.rect {
            hits.push(PlacedControl { rect, action: FixtureChoice::DismissNotice });
        }
        if let Some(rect) = self.root.workspace.chat.composer.field.control.rect {
            hits.push(PlacedControl { rect, action: FixtureChoice::Composer });
        }
        for (_, button, choice) in &self.root.workspace.chat.composer.controls.items {
            use ui::composer::Choice;
            if let Some(rect) = button.control.rect
                && button.control.enabled
            {
                let action = match choice {
                    Choice::RetryCreate => FixtureChoice::RetryCreate,
                    Choice::Attach => FixtureChoice::Attach,
                    Choice::Usage => FixtureChoice::Usage,
                    Choice::Send => FixtureChoice::Send,
                    Choice::Tail => FixtureChoice::Tail,
                    Choice::Queue(op) => FixtureChoice::Queue(op.clone()),
                    Choice::RemoveFile(id) => FixtureChoice::RemoveFile(id.clone()),
                    Choice::Suggest(t) => FixtureChoice::Suggest(t.clone()),
                };
                hits.push(PlacedControl { rect, action });
            }
        }
        for (_, b, a) in &self.root.workspace.chat.transcript.models.controls.items {
            if let Some(r) = b.control.rect
                && b.control.enabled
            {
                let rect = crate::render::intersect(r, b.control.clip);
                if rect.height > 0. {
                    hits.push(PlacedControl {
                        rect,
                        action: match a {
                            ui::composer::ModelChoice::Select(s, m) => FixtureChoice::ChooseModel(s.clone(), m.clone()),
                            ui::composer::ModelChoice::Configure => FixtureChoice::ModelSettings,
                        },
                    });
                }
            }
        }
        if let Some(field) = self.root.workspace.chat.code.view.as_ref().and_then(|v| v.search.as_ref())
            && let Some(rect) = field.control.rect
        {
            hits.push(PlacedControl { rect, action: FixtureChoice::CodeSearch });
        }
        for (_, button, choice) in &self.root.workspace.chat.code.controls.items {
            use code_view::Choice as C;
            if let Some(r) = button.control.rect
                && button.control.enabled
            {
                let rect = crate::render::intersect(r, button.control.clip);
                if rect.height <= 0. || rect.width <= 0. {
                    continue;
                }
                let action = match choice {
                    C::Files => FixtureChoice::Files,
                    C::FileClose => FixtureChoice::FileClose,
                    C::FileChat => FixtureChoice::FileChat,
                    C::FileHidden => FixtureChoice::FileHidden,
                    C::FileSelect(path) => FixtureChoice::FileSelect(path.clone()),
                    C::FileAccept => FixtureChoice::FileAccept,
                    C::FileOpen(p, d) => FixtureChoice::FileOpen(p.clone(), *d),
                    C::FileUp => FixtureChoice::FileUp,
                    C::FileFindHere => FixtureChoice::FileFindHere,
                    C::FileFind => FixtureChoice::FileFind,
                    C::FileClear => FixtureChoice::FileClear,
                    C::FileCopy => FixtureChoice::FileCopy,
                    C::FilePage(next) => FixtureChoice::FilePage(*next),
                };
                hits.push(PlacedControl { rect, action });
            }
        }
        for (_, button, choice) in &self.root.workspace.chat.header.controls.items {
            use ui::header::Choice as C;
            if let Some(rect) = button.control.rect
                && button.control.enabled
            {
                hits.push(PlacedControl {
                    rect,
                    action: match choice {
                        C::Back => FixtureChoice::Back,
                        C::Attachments => FixtureChoice::Attachments,
                        C::Files => FixtureChoice::Files,
                        C::Abort => FixtureChoice::Abort,
                        C::Queue(op) => FixtureChoice::Queue(op.clone()),
                    },
                });
            }
        }
        for row in &self.root.workspace.chat.transcript.rows {
            for part in &row.parts {
                if let (Some(r), Some(open)) = (part.control.rect, part.line.toggle) {
                    let rect = crate::render::intersect(r, part.control.clip);
                    if rect.height > 0. {
                        hits.push(PlacedControl { rect, action: FixtureChoice::Toggle(part.line.key.clone(), !open) });
                    }
                }
            }
        }
        hits
    }
    pub(super) fn test_projects(&self) -> Vec<(Rect, String)> {
        self.placed_controls()
            .into_iter()
            .filter_map(|h| if let FixtureChoice::SelectProject(id) = h.action { Some((h.rect, id)) } else { None })
            .collect()
    }
    pub(super) fn test_chats(&self) -> Vec<(Rect, String)> {
        let mut rows = self
            .placed_controls()
            .into_iter()
            .filter_map(|h| if let FixtureChoice::Select(id) = h.action { Some((h.rect, id)) } else { None })
            .collect::<Vec<_>>();
        if let (Some(rect), Some(session)) =
            (self.root.workspace.chat.header.title.rect, self.controller.account.selected.clone())
        {
            rows.push((rect, session));
        }
        rows
    }
}

impl App {
    /// Fixture setup delegates to the actual retained owner; pointer assertions
    /// still use App::press/release, never these semantic selectors as routing.
    pub(super) fn fixture(&mut self, choice: FixtureChoice) -> Result<()> {
        use FixtureChoice as C;
        match choice {
            C::Select(id) => self.navigate_chat(&id)?,
            C::SelectProject(id) => self.navigate_project(&id)?,
            C::New => self.with_ui(|root, cx| root.workspace.new_chat(cx))?,
            C::NewProject => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::New))?,
            C::RenameProject(id) => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Rename(id)))?,
            C::ProjectPrompt(id) => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Prompt(id)))?,
            C::DeleteProject(id) => self.open_ui(ui::DialogSpec::Topic(ui::TopicEdit::Delete(id)))?,
            C::Settings => self.open_ui(ui::DialogSpec::Connection)?,
            C::AgentSetting(id, command) => {
                self.open_ui(ui::DialogSpec::Operation(ui::Operation::Agent(id, command)))?
            }
            C::Delete(id) => self.open_ui(ui::DialogSpec::Operation(ui::Operation::Delete(id)))?,
            C::Outbox(page) => self.open_ui(ui::DialogSpec::Operation(ui::Operation::Outbox(page)))?,
            C::InspectControl(id) => self.open_ui(ui::DialogSpec::Operation(ui::Operation::Inspect(id)))?,
            C::ForgetControl(id) => self.open_ui(ui::DialogSpec::Operation(ui::Operation::ForgetControl(id)))?,
            C::ReviewRestore(id) => self.open_ui(ui::DialogSpec::Operation(ui::Operation::Review(id)))?,
            C::Confirm => {
                self.ui_event(ui::Event::Submit);
            }
            C::CancelModal | C::Back => self.back(),
            C::Files => self.with_ui(|root, cx| root.workspace.files(cx))?,
            C::FileOpen(..)
            | C::FileUp
            | C::FileClose
            | C::FileChat
            | C::FileHidden
            | C::FileSelect(_)
            | C::FileAccept
            | C::FileFind
            | C::FileFindHere
            | C::FileClear
            | C::FileCopy
            | C::FilePage(_) => self.code_action(choice)?,
            C::Send => self.with_ui(|root, cx| root.workspace.send(cx))?,
            C::Tail => self.with_ui(|root, cx| root.workspace.chat.transcript.tail(cx)),
            C::Composer => {
                let target = self.root.workspace.chat.composer.field.control.target;
                self.ui.focus = Some(target);
                if self.ui.mobile {
                    let id = self.root.workspace.chat.composer.field.editor.native_id();
                    self.with_ui(|_, cx| cx.focus_native(target, id));
                }
            }
            C::Usage => {
                self.root.tooltips.info.dismiss();
                self.root.tooltips.usage.pinned = !self.root.tooltips.usage.pinned;
                self.root.tooltips.usage.suppressed = !self.root.tooltips.usage.pinned;
            }
            _ => anyhow::bail!("This selector is only used to locate a placed control"),
        }
        self.finish_ui_requests()?;
        self.sync_navigation();
        self.reconcile_routes();
        Ok(())
    }
    pub(super) fn code_action(&mut self, action: FixtureChoice) -> Result<()> {
        use code_view::Choice as C;
        let choice = match action {
            FixtureChoice::Files => C::Files,
            FixtureChoice::FileClose => C::FileClose,
            FixtureChoice::FileChat => C::FileChat,
            FixtureChoice::FileHidden => C::FileHidden,
            FixtureChoice::FileSelect(path) => C::FileSelect(path),
            FixtureChoice::FileAccept => C::FileAccept,
            FixtureChoice::FileOpen(p, d) => C::FileOpen(p, d),
            FixtureChoice::FileUp => C::FileUp,
            FixtureChoice::FileFindHere => C::FileFindHere,
            FixtureChoice::FileFind => C::FileFind,
            FixtureChoice::FileClear => C::FileClear,
            FixtureChoice::FileCopy => C::FileCopy,
            FixtureChoice::FilePage(n) => C::FilePage(n),
            _ => return Ok(()),
        };
        let result = self.with_ui(|root, cx| root.workspace.chat.code.code_action(choice, cx));
        if self.root.workspace.chat.code.view.is_some() {
            self.root.workspace.show_chats = false;
            self.root.workspace.attachments.show = false;
        }
        result
    }
    pub(super) fn composer_model_status(&mut self, layer: &mut Layer, summary: Option<&SessionSummary>, rect: Rect) {
        self.with_ui(|root, cx| root.workspace.chat.composer.model_status(cx, layer, summary, rect));
    }
    pub(super) fn notice_frame(&mut self, _ctx: &impl RenderContext, layer: &mut Layer, b: Rect) {
        self.with_ui(|root, cx| root.notice.visit_perframe(&mut ui::Frame { layer, bounds: b, clip: b }, cx));
    }
    pub(super) fn rows(&self, session: &str) -> Vec<Row> {
        projection::rows(&self.controller, session)
    }
    pub(super) fn set_transcript_scroll(&mut self, value: f32) {
        self.with_ui(|root, cx| root.workspace.chat.transcript.set_scroll(value, cx));
    }
    pub(super) fn navigate_chat(&mut self, id: &str) -> Result<()> {
        self.with_ui(|root, cx| root.workspace.navigate_chat(id, cx))
    }
    pub(super) fn navigate_project(&mut self, id: &str) -> Result<()> {
        self.with_ui(|root, cx| root.workspace.navigate_project(id, cx))
    }
}
