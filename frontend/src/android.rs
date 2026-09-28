use crate::{
    app::{App, PlatformAction},
    store::Store,
};
use chad::android::{AndroidApp, Config, Ctx};
use chad::{
    wgpu,
    winit::{
        event::{ElementState, TouchPhase, WindowEvent},
        keyboard::{Key, NamedKey, ModifiersState},
        window::Window,
    },
};
use jni::{
    JNIEnv, JavaVM,
    objects::{JClass, JObject, JString, JValue},
    sys::{jboolean, jint, jlong},
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
    Edit(crate::mobile_input::Edit),
    File(PathBuf, String),
    Paste(u64, String),
    Saved(String, Result<crate::store::SavedDownload,String>),
    Missing(String,String,String,String),
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
        if let NativeEvent::Edit(edit) = &event
            && let Some(NativeEvent::Edit(previous)) = rt.events.last()
            && edit.id == previous.id && edit.revision == previous.revision
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
    modifiers: ModifiersState,
    input: Option<crate::mobile_input::Input>,
    input_menu: bool,
}
impl Android {
    fn java(&mut self, ctx: &Ctx, method: &str, payload: Option<&str>) -> Result<(), String> {
        let vm = unsafe { JavaVM::from_raw(ctx.app.vm_as_ptr().cast()) }.map_err(|e| e.to_string())?;
        let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
        env.with_local_frame(8, |env| {
            let activity = unsafe { JObject::from_raw(ctx.app.activity_as_ptr().cast()) };
            if let Some(payload) = payload {
                let payload = env.new_string(payload)?;
                env.call_method(&activity, method, "(Ljava/lang/String;)V", &[JValue::Object(&payload)])?;
            } else { env.call_method(&activity, method, "()V", &[])?; }
            Ok::<_, jni::errors::Error>(())
        }).map_err(|_| { let _ = env.exception_clear(); "Android input action failed".into() })
    }
    fn sync_input(&mut self, ctx: &Ctx) {
        let input = self.app.native_input();
        let same = match (&self.input, &input) {
            (Some(a), Some(b)) => a.same_configuration(b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            let result = serde_json::to_string(&input).map_err(|e| e.to_string())
                .and_then(|json| self.java(ctx, "syncInput", Some(&json)));
            if result.is_ok() { self.input = input; }
            self.app.report(result.map_err(anyhow::Error::msg));
        }
    }
    fn back(&mut self, ctx: &Ctx) {
        let result = self.java(ctx, "back", None);
        self.app.report(result.map_err(anyhow::Error::msg));
    }
    fn layout(&mut self, ctx: &Ctx) {
        let (width, height) = ctx.size();
        let rect = ctx.app.content_rect();
        let [left, top, right, bottom] = crate::mobile_input::viewport((width, height),
            [rect.left, rect.top, rect.right, rect.bottom],
            [LEFT.load(Ordering::Relaxed), TOP.load(Ordering::Relaxed),
             RIGHT.load(Ordering::Relaxed), BOTTOM.load(Ordering::Relaxed)]);
        if right > left && bottom > top {
            self.app.resize(
                (right - left, bottom - top),
                ctx.window.scale_factor() as f32,
                Vec2::new(left as f32, top as f32),
            );
        }
    }
    fn native_events(&mut self) {
        let events = RUNTIME
            .lock()
            .ok()
            .and_then(|mut g| g.as_mut().map(|r| std::mem::take(&mut r.events)))
            .unwrap_or_default();
        for event in events {
            match event {
                NativeEvent::Back => self.app.back(),
                NativeEvent::Edit(edit) => self.app.native_edit(edit),
                NativeEvent::Paste(token, text) => self.app.paste(token, text),
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
                NativeEvent::Saved(key,result) => self.app.complete_save(&key,result),
                NativeEvent::Missing(identity,lineage,session,entry) => {
                    let result=self.app.controller.forget_download_for(&identity,&lineage,&session,&entry);
                    self.app.report(result.and_then(|_|Err(anyhow::anyhow!("The downloaded file no longer exists. Download it again."))));
                }
                NativeEvent::Error(error) => self.app.report(Err(anyhow::anyhow!(error))),
            }
        }
    }
    fn actions(&mut self, ctx: &Ctx) {
        for action in self.app.actions() {
            let save_key=match &action {PlatformAction::SaveDownload {key,..}=>Some(key.clone()),_=>None};
            let result = (|| -> Result<(), String> {
                let vm = unsafe { JavaVM::from_raw(ctx.app.vm_as_ptr().cast()) }
                    .map_err(|e| e.to_string())?;
                let mut env = vm.attach_current_thread().map_err(|e| e.to_string())?;
                env.with_local_frame(16, |env| {
                    let activity = unsafe { JObject::from_raw(ctx.app.activity_as_ptr().cast()) };
                    match action {
                        PlatformAction::InputMenu => self.input_menu = true,
                        PlatformAction::Haptic => { env.call_method(&activity, "selectionHaptic", "()V", &[])?; }
                        PlatformAction::Background => {
                            env.call_method(&activity, "background", "()V", &[])?;
                        }
                        PlatformAction::PickFile { identity, session } => {
                            self.import_scope = Some((identity, session));
                            env.call_method(&activity, "pickFile", "()V", &[])?;
                        }
                        PlatformAction::Paste { token } => {
                            env.call_method(&activity, "paste", "(J)V", &[JValue::Long(token as i64)])?;
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
                        PlatformAction::SaveDownload {key,source,name} => {
                            let key = env.new_string(key)?;
                            let source = env.new_string(source.to_string_lossy())?;
                            let name = env.new_string(name)?;
                            env.call_method(
                                &activity,
                                "exportFile",
                                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                                &[JValue::Object(&key),JValue::Object(&source),JValue::Object(&name)],
                            )?;
                        }
                        PlatformAction::UseDownload(saved, action, target) => {
                            debug_assert!(matches!(action, crate::app::SavedAction::Open));
                            let reference=env.new_string(saved.reference)?;
                            let mime=env.new_string(saved.mime_type)?;
                            let identity=env.new_string(target.identity)?;
                            let lineage=env.new_string(target.lineage)?;
                            let session=env.new_string(target.session)?;
                            let entry=env.new_string(target.entry)?;
                            env.call_method(&activity,"openSaved",
                                "(Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;Ljava/lang/String;)V",
                                &[JValue::Object(&reference),JValue::Object(&mime),JValue::Object(&identity),
                                  JValue::Object(&lineage),JValue::Object(&session),JValue::Object(&entry)])?;
                        }
                    }
                    Ok::<_, jni::errors::Error>(())
                })
                .map_err(|_| {
                    let _ = env.exception_clear();
                    "Android action failed".to_owned()
                })
            })();
            if let (Some(key),Err(error))=(&save_key,&result) {
                self.app.complete_save(key,Err(error.clone()));
            } else {
                self.app.report(result.map_err(anyhow::Error::msg));
            }
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
            modifiers: ModifiersState::empty(),
            input: None,
            input_menu: false,
        };
        android.layout(ctx);
        Ok(android)
    }
    fn event(&mut self, ctx: &mut Ctx, event: &WindowEvent) {
        self.native_events();
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
            WindowEvent::Focused(focused) => {
                self.app.ui.window_focused = *focused;
                if !focused {
                    self.app.cancel_pointer();
                    let result = self.app.save(); self.app.report(result);
                } else { self.app.ui.dirty = true; }
            }
            WindowEvent::CloseRequested => self.back(ctx),
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let control = (self.modifiers.control_key() && !self.modifiers.alt_key()) || self.modifiers.super_key();
                let shift = self.modifiers.shift_key();
                if matches!(event.logical_key, Key::Named(NamedKey::GoBack | NamedKey::BrowserBack)) {
                    self.back(ctx);
                } else if let Some(key) = crate::keyboard::named(&event.logical_key) {
                    self.app.key(key, control, shift);
                } else if control {
                    if let Some(key) = crate::keyboard::shortcut(event) { self.app.key(key, true, shift); }
                } else if let Some(text) = &event.text && !text.chars().any(char::is_control) {
                    self.app.input(text);
                }
            }
            _ => {}
        }
        self.actions(ctx);
    }
    fn update(&mut self, ctx: &mut Ctx) {
        self.layout(ctx);
        self.native_events();
        if self.app.tick(ctx.dt) {
            ctx.window.request_redraw();
        }
        self.actions(ctx);
    }
    fn suspended(&mut self, _: &mut Ctx) {
        self.native_events();
        self.app.set_connection_visible(false);
        self.app.cancel_pointer();
        let result = self.app.save();
        self.app.report(result);
    }
    fn resumed(&mut self, ctx: &mut Ctx) {
        self.layout(ctx);
        self.app.set_connection_visible(true);
        ctx.window.request_redraw();
    }
    fn frame(&mut self, ctx: &mut Ctx, view: &wgpu::TextureView) {
        self.app.frame(ctx, view);
        self.sync_input(ctx);
        if std::mem::take(&mut self.input_menu) && self.input.is_some() {
            let result = self.java(ctx, "inputMenu", None);
            self.app.report(result.map_err(anyhow::Error::msg));
        }
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
        1 => NativeEvent::File(a.into(), b),
        2 => NativeEvent::Paste(b.parse().unwrap_or(0), a),
        4 => NativeEvent::Saved(a,serde_json::from_str(&b).map_err(|e|e.to_string())),
        5 => NativeEvent::Saved(a,Err(b)),
        6 => match serde_json::from_str::<(String,String,String)>(&b) {
            Ok((lineage,session,entry))=>NativeEvent::Missing(a,lineage,session,entry),
            Err(_) =>NativeEvent::Error("Invalid missing-download response".into()),
        },
        _ => NativeEvent::Error(a),
    });
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_tau_rust_MainActivity_nativeEdit(
    mut env: JNIEnv, _: JClass, id: jlong, revision: jlong, text: JString,
    start: jint, end: jint, composing_start: jint, composing_end: jint,
) {
    if let Ok(text) = env.get_string(&text) {
        queue(NativeEvent::Edit(crate::mobile_input::Edit { id: id as u64, revision: revision as u64,
            text: String::from(text), start, end, composing_start, composing_end }));
    }
}
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_tau_rust_MainActivity_nativeViewport(
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
