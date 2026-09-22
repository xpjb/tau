use crate::{
    app::{App, PlatformAction},
    store::Store,
};
use chad::android::{AndroidApp, Config, Ctx};
use chad::{
    wgpu,
    winit::{
        event::{ElementState, TouchPhase, WindowEvent},
        keyboard::{Key, NamedKey},
        window::Window,
    },
};
use jni::{
    JNIEnv, JavaVM,
    objects::{JClass, JObject, JString, JValue},
    sys::{jboolean, jint},
};
use sanscale::Vec2;
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicI32, Ordering},
    },
};

enum NativeEvent {
    Back,
    Edit(String),
    File(PathBuf, String),
    Paste(String),
    Error(String),
}
struct Runtime {
    window: Weak<Window>,
    events: Vec<NativeEvent>,
}
static RUNTIME: Mutex<Option<Runtime>> = Mutex::new(None);
static LEFT: AtomicI32 = AtomicI32::new(0);
static TOP: AtomicI32 = AtomicI32::new(0);
static RIGHT: AtomicI32 = AtomicI32::new(0);
static BOTTOM: AtomicI32 = AtomicI32::new(0);
fn queue(event: NativeEvent) {
    if let Ok(mut guard) = RUNTIME.lock()
        && let Some(rt) = guard.as_mut()
    {
        // IME full-text updates are snapshots, not deltas; coalesce a busy keyboard.
        if matches!(event, NativeEvent::Edit(_))
            && matches!(rt.events.last(), Some(NativeEvent::Edit(_)))
        {
            rt.events.pop();
        }
        rt.events.push(event);
        if let Some(w) = rt.window.upgrade() {
            w.request_redraw();
        }
    }
}
struct Android {
    app: App,
    import_scope: Option<(String, String)>,
}
impl Android {
    fn layout(&mut self, ctx: &Ctx) {
        let (width, height) = ctx.size();
        let rect = ctx.app.content_rect();
        let valid = rect.right > rect.left && rect.bottom > rect.top;
        let left = LEFT
            .load(Ordering::Relaxed)
            .max(if valid { rect.left } else { 0 })
            .max(0) as u32;
        let top = TOP
            .load(Ordering::Relaxed)
            .max(if valid { rect.top } else { 0 })
            .max(0) as u32;
        let right = width
            .saturating_sub(RIGHT.load(Ordering::Relaxed).max(0) as u32)
            .min(if valid {
                rect.right.max(0) as u32
            } else {
                width
            });
        let bottom = height
            .saturating_sub(BOTTOM.load(Ordering::Relaxed).max(0) as u32)
            .min(if valid {
                rect.bottom.max(0) as u32
            } else {
                height
            });
        if right > left && bottom > top {
            self.app.resize(
                (right - left, bottom - top),
                ctx.window.scale_factor() as f32,
                Vec2::new(left as f32, top as f32),
            );
        }
    }
    fn actions(&mut self, ctx: &Ctx) {
        for action in self.app.actions() {
            let result = (|| -> Result<(), String> {
                let vm = unsafe { JavaVM::from_raw(ctx.app.vm_as_ptr().cast()) }
                    .map_err(|e| e.to_string())?;
                let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
                env.with_local_frame(16, |env| {
                    let activity = unsafe { JObject::from_raw(ctx.app.activity_as_ptr().cast()) };
                    match action {
                        PlatformAction::Background => {
                            env.call_method(&activity, "background", "()V", &[])?;
                        }
                        PlatformAction::PickFile { identity, session } => {
                            self.import_scope = Some((identity, session));
                            env.call_method(&activity, "pickFile", "()V", &[])?;
                        }
                        PlatformAction::Paste => {
                            env.call_method(&activity, "paste", "()V", &[])?;
                        }
                        PlatformAction::Copy(text) => {
                            let text = env.new_string(text)?;
                            env.call_method(
                                &activity,
                                "copy",
                                "(Ljava/lang/String;)V",
                                &[JValue::Object(&text)],
                            )?;
                        }
                        PlatformAction::OpenUrl(url) => {
                            let url = env.new_string(url)?;
                            env.call_method(
                                &activity,
                                "openUrl",
                                "(Ljava/lang/String;)V",
                                &[JValue::Object(&url)],
                            )?;
                        }
                        PlatformAction::Export(path, name) => {
                            let path = env.new_string(path.to_string_lossy())?;
                            let name = env.new_string(name)?;
                            env.call_method(
                                &activity,
                                "exportFile",
                                "(Ljava/lang/String;Ljava/lang/String;)V",
                                &[JValue::Object(&path), JValue::Object(&name)],
                            )?;
                        }
                        PlatformAction::Edit {
                            title,
                            value,
                            secret,
                        } => {
                            let title = env.new_string(title)?;
                            let value = env.new_string(value)?;
                            env.call_method(
                                &activity,
                                "edit",
                                "(Ljava/lang/String;Ljava/lang/String;Z)V",
                                &[
                                    JValue::Object(&title),
                                    JValue::Object(&value),
                                    JValue::Bool(secret as u8),
                                ],
                            )?;
                        }
                    }
                    Ok::<_, jni::errors::Error>(())
                })
                .map_err(|_| {
                    let _ = env.exception_clear();
                    "Android action failed".to_owned()
                })
            })();
            self.app.report(result.map_err(anyhow::Error::msg));
        }
    }
}
impl chad::android::App for Android {
    fn init(ctx: &mut Ctx) -> Result<Self, String> {
        let root = ctx
            .app
            .internal_data_path()
            .ok_or("Private storage unavailable")?;
        *RUNTIME.lock().map_err(|_| "Runtime lock poisoned")? = Some(Runtime {
            window: Arc::downgrade(&ctx.window),
            events: vec![],
        });
        let window = Arc::downgrade(&ctx.window);
        let app = App::new(
            ctx,
            Store::open(root.join("tau2")).map_err(|e| e.to_string())?,
            Arc::new(move || {
                if let Some(w) = window.upgrade() {
                    w.request_redraw();
                }
            }),
            true,
        )
        .map_err(|e| e.to_string())?;
        let mut android = Self {
            app,
            import_scope: None,
        };
        android.layout(ctx);
        Ok(android)
    }
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        self.layout(ctx);
        match event {
            WindowEvent::Touch(t) => {
                let point = Vec2::new(t.location.x as f32, t.location.y as f32);
                match t.phase {
                    TouchPhase::Started => self.app.press(t.id, point, true),
                    TouchPhase::Moved => self.app.motion(t.id, point),
                    TouchPhase::Ended => self.app.release(t.id, point),
                    TouchPhase::Cancelled => self.app.cancel_pointer(),
                }
            }
            WindowEvent::Focused(false) => {
                self.app.cancel_pointer();
                let result = self.app.save();
                self.app.report(result);
            }
            WindowEvent::CloseRequested => self.app.back(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match &event.logical_key {
                    Key::Named(NamedKey::Escape | NamedKey::GoBack | NamedKey::BrowserBack) => {
                        self.app.back()
                    }
                    Key::Named(key) => self.app.key(&format!("{key:?}"), false, false),
                    _ => {
                        if let Some(text) = &event.text {
                            self.app.input(text);
                        }
                    }
                }
            }
            _ => {}
        }
        self.actions(ctx);
    }
    fn update(&mut self, ctx: &mut Ctx) {
        self.layout(ctx);
        let events = RUNTIME
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().map(|r| std::mem::take(&mut r.events)))
            .unwrap_or_default();
        for event in events {
            match event {
                NativeEvent::Back => self.app.back(),
                NativeEvent::Edit(text) => self.app.native_edit(text),
                NativeEvent::Paste(text) => self.app.input(&text),
                NativeEvent::File(path, name) => {
                    let result = if let Some((identity, session)) = self.import_scope.take() {
                        self.app
                            .controller
                            .attach_to(&identity, &session, &path, Some(&name))
                    } else {
                        Err(anyhow::anyhow!(
                            "File picker context expired; choose the file again"
                        ))
                    };
                    self.app.report(result);
                    let _ = std::fs::remove_file(path);
                }
                NativeEvent::Error(error) => self.app.report(Err(anyhow::anyhow!(error))),
            }
        }
        if self.app.tick(ctx.dt) {
            ctx.window.request_redraw();
        }
        self.actions(ctx);
    }
    fn suspended(&mut self, _: &mut Ctx) {
        self.app.cancel_pointer();
        let result = self.app.save();
        self.app.report(result);
    }
    fn resumed(&mut self, ctx: &mut Ctx) {
        self.layout(ctx);
        ctx.window.request_redraw();
    }
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        self.app.frame(ctx, view);
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_tau_rust_MainActivity_nativeBack(_: JNIEnv, _: JClass) -> jboolean {
    queue(NativeEvent::Back);
    1
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_tau_rust_MainActivity_nativeResult(
    mut env: JNIEnv,
    _: JClass,
    kind: jint,
    first: JString,
    second: JString,
) {
    let a = env.get_string(&first).map(String::from).unwrap_or_default();
    let b = env
        .get_string(&second)
        .map(String::from)
        .unwrap_or_default();
    queue(match kind {
        0 => NativeEvent::Edit(a),
        1 => NativeEvent::File(a.into(), b),
        2 => NativeEvent::Paste(a),
        _ => NativeEvent::Error(a),
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_tau_rust_MainActivity_nativeInsets(
    _: JNIEnv,
    _: JClass,
    left: jint,
    top: jint,
    right: jint,
    bottom: jint,
) {
    LEFT.store(left, Ordering::Relaxed);
    TOP.store(top, Ordering::Relaxed);
    RIGHT.store(right, Ordering::Relaxed);
    BOTTOM.store(bottom, Ordering::Relaxed);
    if let Ok(g) = RUNTIME.lock()
        && let Some(w) = g.as_ref().and_then(|r| r.window.upgrade())
    {
        w.request_redraw();
    }
}
#[unsafe(no_mangle)]
pub fn android_main(app: AndroidApp) {
    let crash = app.internal_data_path().map(|p| p.join("client-crash.log"));
    let panic_path = crash.clone();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = &panic_path {
            let trace = format!("{info}\n{}", std::backtrace::Backtrace::force_capture());
            let _ = std::fs::write(path, trace.chars().take(65536).collect::<String>());
        }
    }));
    let result = chad::android::run::<Android>(
        app,
        Config {
            redraw: chad::RedrawMode::OnDemand,
            device_limits: wgpu::Limits {
                max_texture_dimension_2d: 4096,
                ..wgpu::Limits::downlevel_defaults()
            },
            ..Default::default()
        },
    );
    if let Err(error) = result {
        if let Some(path) = crash {
            let _ = std::fs::write(path, error);
        }
    }
    if let Ok(mut runtime) = RUNTIME.lock() {
        *runtime = None;
    }
    // Root Back backgrounds the task; don't pretend process exit fixes winit's
    // documented Destroy/recreation issue (see the branch acceptance notes).
}
