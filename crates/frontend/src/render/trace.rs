//! Bounded in-memory desktop diagnostics. The normal Windows client records
//! scalar timings/counts only: no per-frame allocation, IO, polling or GPU waits.
//! Optional detailed recording adds text-input hashing, never message content.
use super::*;
use serde::Serialize;
use std::{
    cell::Cell,
    collections::{VecDeque, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
    io::{BufWriter, Write},
    time::{Instant, SystemTime, UNIX_EPOCH},
};

const MAX_RECORDS: usize = 4096;
const MAX_SLOW_RECORDS: usize = 32;
const SLOW_US: u64 = 100_000;
thread_local! {
    static ENABLED: Cell<usize> = const { Cell::new(0) };
    static CACHE_WAIT_US: Cell<u64> = const { Cell::new(0) };
}
pub(crate) fn cache_lock_timer() -> Option<Instant> {
    ENABLED.with(|n| n.get() > 0).then(Instant::now)
}
pub(crate) fn cache_lock_wait(started: Option<Instant>) {
    if let Some(started) = started {
        CACHE_WAIT_US.with(|n| n.set(n.get().saturating_add(started.elapsed().as_micros() as u64)));
    }
}
fn cache_wait_us() -> u64 { CACHE_WAIT_US.with(Cell::get) }
#[derive(Clone, Copy)]
pub(crate) struct Stamp { at: Instant, cache_wait_us: u64 }
impl Stamp {
    pub fn now() -> Self { Self { at: Instant::now(), cache_wait_us: cache_wait_us() } }
    pub fn elapsed(self) -> std::time::Duration { self.at.elapsed() }
    fn waited(self) -> u64 { cache_wait_us().saturating_sub(self.cache_wait_us) }
}

#[derive(Clone, Copy, Default, Serialize)]
pub(super) struct Commands {
    pub shape_draws: u32,
    pub shape_vertices: u64,
    pub image_draws: u32,
    pub text_draws: Option<u64>,
    pub emoji_draws: Option<u64>,
    pub skipped_text_segments: u32,
    pub submitted: bool,
}
pub(super) fn text_commands() -> Option<(u64, u64)> {
    #[cfg(any(windows, test))] {
        let work = sanscale::profiling::work_counters();
        Some((work.text_draw_calls, work.emoji_draw_calls))
    }
    #[cfg(not(any(windows, test)))] { None }
}
impl Commands {
    pub fn finish_text(&mut self, before: Option<(u64, u64)>) {
        if let (Some(before), Some(after)) = (before, text_commands()) {
            self.text_draws = Some(after.0.saturating_sub(before.0));
            self.emoji_draws = Some(after.1.saturating_sub(before.1));
        }
    }
}

pub(crate) struct Trace {
    directory: PathBuf,
    header: serde_json::Value,
    detailed: bool,
    started: Instant,
    records: VecDeque<Record>,
    slow: VecDeque<Record>,
    pending_redraw: Option<Instant>,
    last_hover: Option<Instant>,
    discarded: u64,
}
#[derive(Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Stage { Hover, Pointer, Key, Wheel, Ime, Focus, Visibility, Resize, FileDrop, Update, Paint }
#[derive(Clone, Copy, Serialize)]
struct Record {
    elapsed_us: u128,
    #[serde(flatten)]
    event: Event,
}
#[derive(Clone, Copy, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Event {
    Tick { wants_redraw: bool, focused: bool, visible: bool, poll_us: u64, total_us: u64, cache_wait_us: u64 },
    Stage { stage: Stage, frame: u64, callback_us: u64, dirty_after: bool, focused: bool, visible: bool, cache_wait_us: u64 },
    Wake { received: u64, posted: u64 },
    Frame {
        frame: u64,
        size: (u32, u32),
        cpu_render_us: u128,
        redraw_wait_us: Option<u64>,
        layers: usize,
        shapes: usize,
        images: usize,
        text_inputs: usize,
        commands: Commands,
        nonlive_batches: usize,
        cached_blocks: usize,
        #[serde(flatten, skip_serializing_if = "Option::is_none")]
        details: Option<Details>,
    },
}
#[derive(Clone, Copy, Serialize)]
struct Details {
    clipped_text_draws: usize,
    empty_layout_draws: usize,
    nonfinite_text_draws: usize,
    // Geometry/handle/layout-metric fingerprint, NOT text or a pixel hash.
    text_signature: u64,
}
impl Trace {
    pub fn from_env(device: &wgpu::Device, format: wgpu::TextureFormat) -> Option<Self> {
        let path = std::env::var_os("TAU_RENDER_TRACE").filter(|p| !p.is_empty() && p != "off")?;
        Some(Self::new(path.into(), device, format))
    }
    pub(crate) fn new(directory: PathBuf, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self::with_detail(directory, device, format, true)
    }
    #[cfg(any(windows, test))]
    pub(crate) fn normal(directory: PathBuf, device: &wgpu::Device, format: wgpu::TextureFormat) -> Self {
        Self::with_detail(directory, device, format, false)
    }
    fn with_detail(directory: PathBuf, device: &wgpu::Device, format: wgpu::TextureFormat, detailed: bool) -> Self {
        let adapter = device.adapter_info();
        ENABLED.with(|n| n.set(n.get() + 1));
        Self {
            directory,
            header: serde_json::json!({
                "kind": "header", "schema": 3, "version": env!("CARGO_PKG_VERSION"),
                "started_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis(),
                "adapter": adapter.name, "backend": format!("{:?}", adapter.backend),
                "device_type": format!("{:?}", adapter.device_type),
                "vendor": adapter.vendor, "device": adapter.device,
                "driver": adapter.driver, "driver_info": adapter.driver_info,
                "format": format!("{format:?}"), "max_records": MAX_RECORDS, "detailed": detailed,
                "ring_bytes": (MAX_RECORDS + MAX_SLOW_RECORDS) * std::mem::size_of::<Record>(),
                "max_slow_records": MAX_SLOW_RECORDS, "slow_threshold_us": SLOW_US,
            }),
            detailed, started: Instant::now(), records: VecDeque::with_capacity(MAX_RECORDS),
            slow: VecDeque::with_capacity(MAX_SLOW_RECORDS), pending_redraw: None, last_hover: None, discarded: 0,
        }
    }
    fn push(&mut self, event: Event) {
        if self.records.len() == MAX_RECORDS {
            self.records.pop_front();
            self.discarded += 1;
        }
        let record = Record { elapsed_us: self.started.elapsed().as_micros(), event };
        let slow = match event {
            Event::Stage { callback_us, .. } => callback_us >= SLOW_US,
            Event::Tick { total_us, .. } => total_us >= SLOW_US,
            Event::Frame { cpu_render_us, redraw_wait_us, .. } => cpu_render_us >= u128::from(SLOW_US)
                || redraw_wait_us.is_some_and(|us| us >= SLOW_US),
            Event::Wake { .. } => false,
        };
        if slow {
            if self.slow.len() == MAX_SLOW_RECORDS { self.slow.pop_front(); }
            self.slow.push_back(record);
        }
        self.records.push_back(record);
    }
    pub fn tick(&mut self, wants_redraw: bool, focused: bool, visible: bool, poll_us: u64, started: Stamp) {
        if wants_redraw { self.pending_redraw.get_or_insert_with(Instant::now); }
        self.push(Event::Tick { wants_redraw, focused, visible, poll_us,
            total_us: started.elapsed().as_micros() as u64, cache_wait_us: started.waited() });
    }
    pub fn stage(&mut self, stage: Stage, frame: u64, started: Stamp, (wants_redraw, focused, visible): (bool, bool, bool)) {
        if matches!(stage, Stage::Hover) {
            if self.last_hover.is_some_and(|at| at.elapsed().as_millis() < 50) { return; }
            self.last_hover = Some(Instant::now());
        }
        if wants_redraw { self.pending_redraw.get_or_insert_with(Instant::now); }
        self.push(Event::Stage { stage, frame, callback_us: started.elapsed().as_micros() as u64,
            dirty_after: wants_redraw, focused, visible, cache_wait_us: started.waited() });
    }
    pub fn wakes(&mut self, received: u64, posted: u64) {
        if received > 0 { self.push(Event::Wake { received, posted }); }
    }
    pub(super) fn frame(&mut self, ctx: &impl RenderContext, text: &TextService,
        layers: &[PaintBatch<'_>], batches: &[sanscale::Batch], commands: Commands, started: Instant) {
        // The normal path does NOT walk/hash/measure Draws. Batch validity reads
        // only the already-prepared batch's handle/revision list (no reshaping).
        let details = self.detailed.then(|| {
            let mut signature = DefaultHasher::new();
            let mut clipped_text_draws = 0;
            let mut empty_layout_draws = 0;
            let mut nonfinite_text_draws = 0;
            let (width, height) = ctx.size();
            for layer in layers {
                layer.draws.len().hash(&mut signature);
                for draw in layer.draws {
                    draw.block.hash(&mut signature);
                    draw.paint.hash(&mut signature);
                    let rect = draw.clip.unwrap_or(Rect::new(0., 0., width as f32, height as f32));
                    let values = [draw.at.x, draw.at.y, draw.size, rect.x, rect.y, rect.width, rect.height,
                        draw.color.0[0], draw.color.0[1], draw.color.0[2], draw.color.0[3]];
                    values.map(f32::to_bits).hash(&mut signature);
                    nonfinite_text_draws += usize::from(values.iter().any(|v| !v.is_finite()));
                    clipped_text_draws += usize::from(scissor(rect, width, height).is_none());
                    let layout = text.measure(draw.block);
                    empty_layout_draws += usize::from(layout.line_count() == 0);
                    (layout.len_bytes(), layout.line_count(), layout.width_em().to_bits(), layout.height_em().to_bits())
                        .hash(&mut signature);
                }
            }
            Details { clipped_text_draws, empty_layout_draws, nonfinite_text_draws, text_signature: signature.finish() }
        });
        let redraw_wait_us = self.pending_redraw.take().map(|at| at.elapsed().as_micros() as u64);
        self.push(Event::Frame {
            redraw_wait_us, frame: ctx.frame_index(), size: ctx.size(), cpu_render_us: started.elapsed().as_micros(),
            layers: layers.len(), shapes: layers.iter().map(|l| l.rects.len()).sum(),
            images: layers.iter().map(|l| l.images.len()).sum(),
            text_inputs: layers.iter().map(|l| l.draws.len()).sum(), commands,
            nonlive_batches: batches.iter().filter(|b| !text.batch_live(b)).count(),
            cached_blocks: text.diagnostics().cache_occupancy().1, details,
        });
    }
    fn write(&self, out: &mut impl Write) -> std::io::Result<()> {
        serde_json::to_writer(&mut *out, &self.header)?;
        writeln!(out)?;
        serde_json::to_writer(&mut *out, &serde_json::json!({"kind":"capture", "discarded_records":self.discarded}))?;
        writeln!(out)?;
        for record in &self.slow {
            serde_json::to_writer(&mut *out, &serde_json::json!({"kind":"slow", "sample":record}))?;
            writeln!(out)?;
        }
        for record in &self.records {
            serde_json::to_writer(&mut *out, record)?;
            writeln!(out)?;
        }
        out.flush()
    }
    /// User-requested snapshots are unique. Automatic shutdown replaces one
    /// bounded latest.jsonl, never an ever-growing collection of session logs.
    pub fn save(&self) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(&self.directory)?;
        let path = self.directory.join(format!("render-{}.jsonl", uuid::Uuid::new_v4()));
        let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        self.write(&mut BufWriter::new(file))?;
        Ok(path)
    }
    fn save_latest(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.directory)?;
        let mut file = tempfile::NamedTempFile::new_in(&self.directory)?;
        self.write(&mut BufWriter::new(file.as_file_mut()))?;
        file.persist(self.directory.join("latest.jsonl")).map_err(|e| e.error)?;
        Ok(())
    }
}
impl Drop for Trace {
    fn drop(&mut self) {
        ENABLED.with(|n| n.set(n.get().saturating_sub(1)));
        if let Err(error) = self.save_latest() { log::warn!("Could not save rendering diagnostics: {error}"); }
    }
}

#[cfg(all(test, not(target_os = "android")))]
#[path = "../../tests/unit/render/trace.rs"]
mod tests;
