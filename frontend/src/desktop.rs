use crate::{
    app::{App, ConnectionPreview, PlatformAction},
    store::{Settings, Store},
};
use chad::winit::{
    dpi::{PhysicalPosition, PhysicalSize},
    event::{ElementState, Ime, MouseButton, MouseScrollDelta, WindowEvent},
    keyboard::{Key, ModifiersState, NamedKey},
    window::CursorIcon,
};
use chad::{ChadApp, Config, Ctx, wgpu};
use sanscale::Vec2;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
};

// Account/session are captured when the document picker is opened.
type PickResult = Result<(String, String, Vec<PathBuf>), String>;
struct Desktop {
    app: App,
    cursor: Vec2,
    cursor_icon: CursorIcon,
    modifiers: ModifiersState,
    clipboard: Option<arboard::Clipboard>,
    rx: mpsc::Receiver<PickResult>,
    tx: mpsc::Sender<PickResult>,
}
fn root() -> PathBuf {
    if let Some(path) = std::env::var_os("TAU2_DATA_DIR") {
        return path.into();
    }
    std::env::var_os("LOCALAPPDATA")
        .or_else(|| std::env::var_os("XDG_DATA_HOME"))
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("Tau2")
}
impl ChadApp for Desktop {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        #[cfg(windows)]
        {
            use chad::winit::{
                platform::windows::{IconExtWindows, WindowExtWindows},
                window::Icon,
            };
            let icon = Icon::from_resource(1, None).map_err(|e| e.to_string())?;
            ctx.window.set_taskbar_icon(Some(icon));
        }
        let store = Store::open(root()).map_err(|e| e.to_string())?;
        if let (Ok(server_url), Ok(token)) =
            (std::env::var("TAU2_SERVER"), std::env::var("TAU2_TOKEN"))
        {
            store
                .put("", "settings", &Settings { server_url, token })
                .map_err(|e| e.to_string())?;
        }
        let waker = ctx.waker();
        let app = App::new(ctx, store, Arc::new(move || waker.wake()), false)
            .map_err(|e| e.to_string())?;
        ctx.window.set_ime_allowed(true);
        let (tx, rx) = mpsc::channel();
        Ok(Self {
            app,
            cursor: Vec2::new(0., 0.),
            cursor_icon: CursorIcon::Default,
            modifiers: ModifiersState::empty(),
            clipboard: arboard::Clipboard::new().ok(),
            rx,
            tx,
        })
    }
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        match event {
            WindowEvent::CloseRequested => {
                let result = self.app.save();
                if result.is_ok() {
                    ctx.exit();
                } else {
                    self.app.report(result);
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor = Vec2::new(position.x as f32, position.y as f32);
                self.app.hover(Some(self.cursor));
                self.app.motion(0, self.cursor);
            }
            WindowEvent::CursorLeft { .. } => self.app.hover(None),
            WindowEvent::Occluded(occluded) => self.app.set_connection_visible(!occluded),
            WindowEvent::ScaleFactorChanged { .. } | WindowEvent::Focused(true) => {
                ctx.request_redraw()
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Left,
                ..
            } => {
                if *state == ElementState::Pressed {
                    if !self.app.cancel_autoscroll() {
                        self.app.press(0, self.cursor, false);
                    }
                } else {
                    self.app.release(0, self.cursor);
                }
            }
            WindowEvent::MouseInput {
                state,
                button: MouseButton::Middle,
                ..
            } => self
                .app
                .middle(*state == ElementState::Pressed, self.cursor),
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Right,
                ..
            } => {
                self.app.context_at(self.cursor);
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let (x, y) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * 48. * ctx.scale_factor() as f32, -y * 48. * ctx.scale_factor() as f32),
                    MouseScrollDelta::PixelDelta(p) => (-p.x as f32, -p.y as f32),
                };
                let horizontal = x.abs() > y.abs();
                self.app.wheel(if horizontal { x } else { y }, horizontal || self.modifiers.shift_key(), self.cursor);
            }
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::Ime(Ime::Commit(text)) => self.app.input(text),
            WindowEvent::Ime(Ime::Disabled) => self.app.cancel_preedit(),
            WindowEvent::Ime(Ime::Preedit(text, cursor)) => self.app.preedit(text.clone(), *cursor),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let cancelled = self.app.cancel_autoscroll();
                if cancelled && event.logical_key == Key::Named(NamedKey::Escape) {
                    self.sync_cursor(ctx);
                    ctx.request_redraw();
                    return;
                }
                let control = (self.modifiers.control_key() && !self.modifiers.alt_key())
                    || self.modifiers.super_key();
                let shift = self.modifiers.shift_key();
                if let Some(key) = crate::keyboard::named(&event.logical_key) {
                    self.app.key(key, control, shift);
                } else if control {
                    if let Some(key) = crate::keyboard::shortcut(event) {
                        self.app.key(key, true, shift);
                    }
                } else if !self.app.composing()
                    && let Some(text) = &event.text
                    && !text.chars().any(char::is_control)
                {
                    self.app.input(text);
                }
            }
            WindowEvent::DroppedFile(path) => {
                let result = self.app.controller.attach(path, None);
                self.app.report(result);
            }
            WindowEvent::Focused(false) => {
                self.modifiers = ModifiersState::empty();
                self.app.cancel_preedit();
                self.app.cancel_pointer();
                let result = self.app.save();
                self.app.report(result);
            }
            _ => {}
        }
        self.actions(ctx);
        self.sync_cursor(ctx);
        // Desktop OnDemand forwards input but does not schedule a frame for it.
        // Waiting until update() to request redraw deadlocks visible input feedback.
        if self.app.needs_redraw() {
            ctx.request_redraw();
        }
    }
    fn update(&mut self, ctx: &mut Ctx) {
        self.app
            .resize(ctx.size(), ctx.scale_factor() as f32, Vec2::new(0., 0.));
        while let Ok(result) = self.rx.try_recv() {
            match result {
                Ok((identity, session, paths)) => {
                    for path in paths {
                        let result = self
                            .app
                            .controller
                            .attach_to(&identity, &session, &path, None);
                        self.app.report(result);
                    }
                }
                Err(error) => self.app.report(Err(anyhow::anyhow!(error))),
            }
        }
        if self.app.tick(ctx.dt) {
            ctx.request_redraw();
        }
        self.actions(ctx);

    }
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        self.app.frame(ctx, view);
        if let Some(rect) = self.app.ime_rect() {
            ctx.window.set_ime_cursor_area(
                PhysicalPosition::new(rect.x as f64, rect.y as f64),
                PhysicalSize::new(rect.width.ceil() as u32, rect.height.ceil() as u32),
            );
        }
        self.sync_cursor(ctx);
        if self.app.needs_redraw() {
            ctx.request_redraw();
        }
    }
}
impl Desktop {
    fn sync_cursor(&mut self, ctx: &Ctx) {
        let icon = self.app.cursor();
        if icon != self.cursor_icon {
            self.cursor_icon = icon;
            ctx.window.set_cursor(icon);
        }
    }
    fn actions(&mut self, ctx: &mut Ctx) {
        for action in self.app.actions() {
            match action {
                PlatformAction::Copy(text) => {
                    if let Some(c) = &mut self.clipboard {
                        let result = c.set_text(text).map_err(anyhow::Error::from);
                        self.app.report(result);
                    }
                }
                PlatformAction::Paste => {
                    if let Some(c) = &mut self.clipboard {
                        if let Ok(text) = c.get_text() {
                            self.app.input(&text);
                        } else if let Ok(image) = c.get_image() {
                            let path = self.app.controller.store.root.join("clipboard.png");
                            let result = image::save_buffer(
                                &path,
                                &image.bytes,
                                image.width as u32,
                                image.height as u32,
                                image::ColorType::Rgba8,
                            )
                            .map_err(anyhow::Error::from)
                            .and_then(|_| self.app.controller.attach(&path, Some("clipboard.png")));
                            self.app.report(result);
                        }
                    }
                }
                PlatformAction::PickFile { identity, session } => {
                    let tx = self.tx.clone();
                    let waker = ctx.waker();
                    std::thread::spawn(move || {
                        if let Some(files) = rfd::FileDialog::new().pick_files() {
                            let _ = tx.send(Ok((identity, session, files)));
                            waker.wake();
                        }
                    });
                }
                PlatformAction::Export(source, name) => {
                    let tx = self.tx.clone();
                    let waker = ctx.waker();
                    std::thread::spawn(move || {
                        let safe = Path::new(&name)
                            .file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| "download".into());
                        if let Some(path) = rfd::FileDialog::new().set_file_name(safe).save_file() {
                            let result = std::fs::copy(&source, &path)
                                .and_then(|_| std::fs::OpenOptions::new().write(true).open(&path)?.sync_all());
                            if let Err(e) = result {
                                let _ = tx.send(Err(e.to_string()));
                                waker.wake();
                            }
                        }
                    });
                }
                PlatformAction::OpenUrl(url) => {
                    if let Err(e) = open::that(url) {
                        self.app.report(Err(e.into()));
                    }
                }
                PlatformAction::Background => {}
                PlatformAction::Edit {
                    title,
                    value,
                    secret,
                    single_line,
                } => {
                    let _ = (title, value, secret, single_line);
                }
            }
        }
    }
}
pub fn run() -> Result<(), String> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("--screenshot") {
        let preview = args.iter().position(|a| a == "--connection-preview")
            .map(|i| match args.get(i + 1).map(String::as_str) {
                Some("received") => Ok(ConnectionPreview::Received),
                Some("disconnected") => Ok(ConnectionPreview::Disconnected),
                Some("unconfigured") => Ok(ConnectionPreview::Unconfigured),
                Some("waiting" | "--phone") | None => Ok(ConnectionPreview::Waiting),
                Some(mode) => Err(format!("Unknown connection preview: {mode}")),
            }).transpose()?;
        return screenshot(
            Path::new(args.get(1).ok_or("Missing output path")?),
            args.iter().any(|a| a == "--phone"),
            preview,
        );
    }
    #[cfg(windows)]
    {
        // Keep beta taskbar grouping/pins separate from the stable Tau launcher.
        // SAFETY: the application ID is a static, null-terminated UTF-16 string.
        let result = unsafe {
            windows_sys::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID(
                windows_sys::core::w!("app.tau.beta"),
            )
        };
        if result < 0 {
            return Err(format!(
                "Could not set Tau Beta application identity: {result:#x}"
            ));
        }
    }
    let icon = image::load_from_memory(include_bytes!("../assets/tau-beta.png"))
        .map_err(|e| e.to_string())?
        .to_rgba8();
    chad::run::<Desktop>(Config {
        title: "Tau Beta".into(),
        icon: Some(chad::AppIcon {
            width: icon.width(),
            height: icon.height(),
            rgba: icon.into_raw(),
        }),
        size: (1100, 760),
        redraw: chad::RedrawMode::OnDemand,
        device_limits: limits(),
        ..Default::default()
    })
}
pub fn limits() -> wgpu::Limits {
    wgpu::Limits {
        max_texture_dimension_2d: 4096,
        ..wgpu::Limits::downlevel_defaults()
    }
}
pub fn screenshot(path: &Path, phone: bool, connection_preview: Option<ConnectionPreview>) -> Result<(), String> {
    let size = if phone { (1080, 2160) } else { (1280, 900) };
    let ctx = chad::HeadlessCtx::new(&Config {
        size,
        device_limits: limits(),
        ..Default::default()
    })?;
    let root = std::env::temp_dir().join(format!("tau-preview-{}", uuid::Uuid::new_v4()));
    let store = Store::open(root.clone()).map_err(|e| e.to_string())?;
    let mut app = App::new(&ctx, store, Arc::new(|| {}), phone).map_err(|e| e.to_string())?;
    app.back(); // The offline fixture bypasses first-run connection setup.
    crate::demo::populate(&mut app.controller).map_err(|e| e.to_string())?;
    app.resize(size, if phone { 2.5 } else { 1. }, Vec2::new(0., 0.));
    app.tick(0.);
    if let Some(preview) = connection_preview { app.preview_connection(preview); }
    app.frame(&ctx, ctx.view());
    let rgba = ctx.read_rgba8()?;
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(file, size.0, size.1);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    encoder
        .write_header()
        .map_err(|e| e.to_string())?
        .write_image_data(&rgba)
        .map_err(|e| e.to_string())?;
    drop(app);
    let _ = std::fs::remove_dir_all(root);
    Ok(())
}
