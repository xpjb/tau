//! The shared composer/settings/dialog editor. Sanscale owns visual caret
//! geometry; this component owns text, selection, undo, composition and viewport.
use crate::render::{Layer, Renderer, color, contains};
use sanscale::{Align, Boundaries, Caret, Draw, FontChainHandle, Layout, Motion, Rect, ShapedHandle, Style, TextService, Vec2};
use std::{borrow::Cow, ops::Range};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Clone)]
struct Snapshot {
    value: String,
    caret: Caret,
    anchor: usize,
}
struct Composition {
    text: String,
    cursor: Option<Range<usize>>,
    replace: Range<usize>,
}
#[derive(Clone, Copy)]
struct View {
    rect: Rect,
    size: f32,
    secret: bool,
}
impl View {
    fn inner(self) -> Rect {
        let pad = self.size * 0.75;
        Rect::new(self.rect.x + pad, self.rect.y + pad,
            (self.rect.width - pad * 2.).max(1.), (self.rect.height - pad * 2.).max(1.))
    }
}
struct CachedLayout {
    style: Style,
    secret: bool,
    block: ShapedHandle,
}
pub struct Editor {
    // Mutate through the editor operations, not by assigning text behind its cache.
    pub value: String,
    pub single_line: bool,
    center_one_line: bool,
    caret: Caret,
    anchor: usize,
    goal: Option<f32>,
    after_edit: bool,
    composition: Option<Composition>,
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    layout: Option<CachedLayout>,
    view: Option<View>,
    visible: bool,
    scroll: Vec2, // em-space, independent of transcript scrolling and caret position
    follow_caret: bool,
}
impl Editor {
    pub fn new(value: String) -> Self {
        let value = normalize(&value, false);
        let end = value.len();
        Self {
            value, caret: Caret { byte_index: end, line_index: 0 }, anchor: end,
            single_line: false, center_one_line: false, goal: None, after_edit: true, follow_caret: true,
            composition: None, undo: Vec::new(), redo: Vec::new(), layout: None,
            view: None, visible: false, scroll: Vec2::new(0., 0.),
        }
    }
    pub fn line(value: String) -> Self {
        Self { single_line: true, ..Self::new(normalize(&value, true)) }
    }
    /// A multiline composer centers its text only while it occupies one visual
    /// line. Other multiline fields (such as large settings editors) stay top-aligned.
    pub fn composer(value: String) -> Self {
        Self { center_one_line: true, ..Self::new(value) }
    }
    pub fn range(&self) -> Range<usize> {
        self.caret.byte_index.min(self.anchor)..self.caret.byte_index.max(self.anchor)
    }
    pub fn selected(&self) -> &str { &self.value[self.range()] }
    fn snapshot(&self) -> Snapshot {
        Snapshot { value: self.value.clone(), caret: self.caret, anchor: self.anchor }
    }
    /// Returns true only for a content change, not selection/preedit changes.
    pub fn replace(&mut self, value: &str) -> bool {
        let range = self.composition.as_ref().map_or_else(|| self.range(), |c| c.replace.clone());
        self.replace_range(range, value)
    }
    fn replace_range(&mut self, range: Range<usize>, value: &str) -> bool {
        let value = normalize(value, self.single_line);
        if self.value.len() - range.len() + value.len() > tau_protocol::MAX_REQUEST_BYTES {
            return false;
        }
        let changed = self.value[range.clone()] != value;
        if changed {
            self.undo.push(self.snapshot());
            if self.undo.len() > 64 { self.undo.remove(0); }
            self.redo.clear();
            self.value.replace_range(range.clone(), &value);
        }
        let inserted_end = range.start + value.len();
        // An insertion can join the following combining mark/ZWJ sequence.
        self.caret.byte_index = self.value.grapheme_indices(true).map(|(at, _)| at)
            .find(|&at| at >= inserted_end).unwrap_or(self.value.len());
        self.anchor = self.caret.byte_index;
        self.composition = None;
        self.layout = None;
        self.after_edit = true;
        self.goal = None;
        self.follow_caret = true;
        changed
    }
    #[cfg(target_os = "android")]
    pub fn replace_all(&mut self, value: &str) -> bool {
        self.replace_range(0..self.value.len(), value)
    }
    fn undo(&mut self, redo: bool) -> bool {
        let previous = if redo { self.redo.pop() } else { self.undo.pop() };
        let Some(previous) = previous else { return false; };
        let current = self.snapshot();
        if redo { self.undo.push(current); } else { self.redo.push(current); }
        self.value = previous.value;
        self.caret = previous.caret;
        self.anchor = previous.anchor;
        self.composition = None;
        self.layout = None;
        self.after_edit = false;
        self.goal = None;
        self.follow_caret = true;
        true
    }
    pub fn composing(&self) -> bool { self.composition.is_some() }
    pub fn preedit(&mut self, text: String, cursor: Option<(usize, usize)>) {
        if text.is_empty() {
            self.composition = None;
        } else {
            let replace = self.composition.as_ref().map_or_else(|| self.range(), |c| c.replace.clone());
            let cursor = cursor.map(|(a, b)| {
                // Winit offsets address the preedit's UTF-8 bytes, before line normalization.
                let map = |at| normalize(&text[..char_boundary(&text, at)], self.single_line).len();
                map(a)..map(b)
            });
            self.composition = Some(Composition {
                text: normalize(&text, self.single_line), cursor, replace,
            });
        }
        self.layout = None;
        self.goal = None;
        self.follow_caret = true;
    }
    fn display(&self) -> (Cow<'_, str>, usize) {
        match &self.composition {
            Some(c) => (Cow::Owned(format!("{}{}{}", &self.value[..c.replace.start], c.text,
                &self.value[c.replace.end..])), c.replace.start + c.cursor.as_ref().map_or(c.text.len(), |r| r.end)),
            None => (Cow::Borrowed(&self.value), self.caret.byte_index),
        }
    }
    fn display_byte(&self, byte: usize, secret: bool) -> usize {
        if secret { self.value[..byte].graphemes(true).count() * "•".len() } else { byte }
    }
    fn source_byte(&self, byte: usize, secret: bool) -> usize {
        if secret {
            self.value.grapheme_indices(true).nth(byte / "•".len()).map_or(self.value.len(), |(at, _)| at)
        } else { char_boundary(&self.value, byte) }
    }
    fn placed(&self, layout: &Layout, secret: bool) -> Caret {
        if self.composition.is_some() {
            let (display, byte) = self.display();
            let byte = if secret { display[..byte].graphemes(true).count() * "•".len() } else { byte };
            layout.caret_after_edit(byte)
        } else {
            let caret = Caret { byte_index: self.display_byte(self.caret.byte_index, secret), ..self.caret };
            if self.after_edit { layout.caret_after_edit(caret.byte_index) } else { layout.clamp_caret(caret) }
        }
    }
    fn block(&mut self, text: &mut TextService, chain: FontChainHandle, width: f32, size: f32, secret: bool) -> Option<ShapedHandle> {
        let style = Style { chain, wrap_em: (!self.single_line).then_some(width / size), align: Align::Left, line_spacing: 1.2 };
        if let Some(cached) = &self.layout
            && cached.style == style && cached.secret == secret
            && text.measure(cached.block).line_count() > 0
        { return Some(cached.block); }
        let (display, _) = self.display();
        let display = if secret { Cow::Owned("•".repeat(display.graphemes(true).count())) } else { display };
        let block = text.shape_transient(&display, &style)?;
        drop(display);
        self.layout = Some(CachedLayout { style, secret, block });
        if self.composition.is_none() {
            self.caret.line_index = self.placed(text.measure(block), secret).line_index;
            self.after_edit = false;
        }
        self.follow_caret = true;
        Some(block)
    }
    fn prepare_view(&mut self, text: &mut TextService, chain: FontChainHandle, follow: bool) -> Option<ShapedHandle> {
        let view = self.view?;
        let block = self.block(text, chain, view.inner().width, view.size, view.secret)?;
        let layout = text.measure(block);
        self.clamp_scroll(layout, view);
        if follow && self.follow_caret {
            let caret = layout.caret_rect(self.placed(layout, view.secret));
            self.scroll.x = reveal(self.scroll.x, caret.x_em, 1.5 / view.size, view.inner().width / view.size);
            self.scroll.y = reveal(self.scroll.y, caret.y_em, caret.height_em, view.inner().height / view.size);
            self.clamp_scroll(layout, view);
            self.follow_caret = false;
        }
        Some(block)
    }
    fn clamp_scroll(&mut self, layout: &Layout, view: View) {
        self.scroll.x = self.scroll.x.clamp(0., (layout.width_em() + 1.5 / view.size - view.inner().width / view.size).max(0.));
        self.scroll.y = self.scroll.y.clamp(0., (layout.height_em() - view.inner().height / view.size).max(0.));
    }
    fn origin(&self, layout: &Layout, view: View) -> Vec2 {
        let inner = view.inner();
        let center = if self.single_line || (self.center_one_line && layout.line_count() == 1) {
            ((inner.height - layout.height_em() * view.size) * 0.5).max(0.)
        } else { 0. };
        Vec2::new(inner.x - self.scroll.x * view.size, inner.y + center - self.scroll.y * view.size)
    }
    pub fn hide(&mut self) { self.visible = false; }
    pub fn contains(&self, point: Vec2) -> bool { self.visible && self.view.is_some_and(|v| contains(v.rect, point)) }
    fn place(&mut self, caret: Caret, secret: bool, extend: bool) {
        self.caret = Caret { byte_index: self.source_byte(caret.byte_index, secret), ..caret };
        if !extend { self.anchor = self.caret.byte_index; }
        self.after_edit = false;
        self.follow_caret = true;
    }
    /// One input adapter for all fields. App owns send/confirm/focus/clipboard policy.
    /// Return true only when draft persistence is needed.
    pub fn key(&mut self, text: &mut TextService, chain: FontChainHandle, key: &str, ctrl: bool, shift: bool) -> bool {
        if self.composing() { return false; }
        match key {
            "a" | "A" if ctrl => {
                self.anchor = 0;
                self.caret.byte_index = self.value.len();
                self.after_edit = true;
                self.goal = None;
                self.follow_caret = true;
                return false;
            }
            "z" | "Z" if ctrl => return self.undo(shift),
            "y" | "Y" if ctrl => return self.undo(true),
            "Space" if !ctrl => return self.replace(" "),
            "Enter" => return self.replace("\n"),
            "Tab" => return self.replace("    "),
            _ => {}
        }
        let Some(view) = self.view else { return false; };
        let Some(block) = self.prepare_view(text, chain, false) else { return false; };
        let layout = text.measure(block);
        let caret = self.placed(layout, view.secret);
        let page = (view.inner().height / view.size / layout.caret_rect(caret).height_em.max(0.01)).floor().max(1.) as usize;
        let motion = match key {
            "ArrowLeft" | "Backspace" => if ctrl { Motion::WordLeft } else { Motion::Left },
            "ArrowRight" | "Delete" => if ctrl { Motion::WordRight } else { Motion::Right },
            "ArrowUp" => Motion::Up,
            "ArrowDown" => Motion::Down,
            "Home" => if ctrl { Motion::DocStart } else { Motion::Home },
            "End" => if ctrl { Motion::DocEnd } else { Motion::End },
            "PageUp" => Motion::PageUp(page),
            "PageDown" => Motion::PageDown(page),
            _ => return false,
        };
        let selection = self.range();
        let deleting = matches!(key, "Backspace" | "Delete");
        if deleting && !selection.is_empty() { return self.replace(""); }
        if deleting && !ctrl {
            // Editing is grapheme-based, not glyph-based: a font's fi ligature
            // must never turn Backspace into deleting two letters. Geometry and
            // visual motion still use the engine's caret stops.
            let byte = self.caret.byte_index;
            let range = if key == "Backspace" {
                self.value[..byte].grapheme_indices(true).next_back().map_or(byte, |(at, _)| at)..byte
            } else {
                byte..self.value[byte..].graphemes(true).next().map_or(byte, |g| byte + g.len())
            };
            return self.replace_range(range, "");
        }
        if !deleting && !shift && !selection.is_empty() && matches!(key, "ArrowLeft" | "ArrowRight") {
            let byte = if key == "ArrowLeft" { selection.start } else { selection.end };
            let caret = layout.caret_at(self.display_byte(byte, view.secret));
            self.goal = None;
            self.place(caret, view.secret, false);
            return false;
        }
        let words = Words { text: &self.value, secret_len: view.secret.then_some(layout.len_bytes()) };
        let next = layout.caret_move(caret, motion, &mut self.goal, &words);
        if deleting {
            let byte = self.source_byte(next.byte_index, view.secret);
            self.replace_range(byte.min(self.caret.byte_index)..byte.max(self.caret.byte_index), "")
        } else {
            self.place(next, view.secret, shift);
            false
        }
    }
    // Mouse/wheel operate on the displayed viewport, not a pending keyboard
    // reveal. Repainting is the point where that reveal becomes visible.
    pub fn hit(&mut self, text: &mut TextService, chain: FontChainHandle, point: Vec2, extend: bool) {
        if self.composing() { self.preedit(String::new(), None); }
        let Some(view) = self.view else { return; };
        let Some(block) = self.prepare_view(text, chain, false) else { return; };
        let layout = text.measure(block);
        let origin = self.origin(layout, view);
        if let Some(caret) = layout.hit_test(Vec2::new((point.x - origin.x) / view.size, (point.y - origin.y) / view.size)) {
            self.goal = None;
            self.place(caret, view.secret, extend);
        }
    }
    pub fn wheel(&mut self, text: &mut TextService, chain: FontChainHandle, amount: f32, horizontal: bool) -> bool {
        let Some(view) = self.view else { return false; };
        let Some(block) = self.prepare_view(text, chain, false) else { return false; };
        let before = self.scroll;
        if horizontal || self.single_line { self.scroll.x += amount / view.size; }
        else { self.scroll.y += amount / view.size; }
        self.clamp_scroll(text.measure(block), view);
        self.follow_caret = false;
        self.scroll != before
    }
    pub fn drag_scroll(&mut self, text: &mut TextService, chain: FontChainHandle, point: Vec2, dt: f32) -> bool {
        let Some(view) = self.view else { return false; };
        let inner = view.inner();
        let speed = |p: f32, start: f32, extent: f32| {
            let distance = if p < start { p - start } else { (p - start - extent).max(0.) };
            (distance * 12.).clamp(-1200. * view.size / 16., 1200. * view.size / 16.) * dt.min(0.05)
        };
        let x = speed(point.x, inner.x, inner.width);
        let y = speed(point.y, inner.y, inner.height);
        let changed = self.wheel(text, chain, x, true) | self.wheel(text, chain, y, false);
        if changed { self.hit(text, chain, point, true); }
        changed
    }
    pub fn height(&mut self, renderer: &mut Renderer, width: f32, size: f32) -> f32 {
        let inner = View { rect: Rect::new(0., 0., width, 56.), size, secret: false }.inner();
        let height = self.block(&mut renderer.text, renderer.faces.prose[0], inner.width, size, false)
            .map_or(0., |b| renderer.text.measure(b).height_em() * size);
        (height + size * 1.5).clamp(size * 3.5, size * 10.)
    }
    pub fn ime_rect(&self, text: &TextService) -> Option<Rect> {
        if !self.visible { return None; }
        let view = self.view?;
        let layout = text.measure(self.layout.as_ref()?.block);
        let caret = layout.caret_rect(self.placed(layout, view.secret));
        let origin = self.origin(layout, view);
        let inner = view.inner();
        Some(Rect::new((origin.x + caret.x_em * view.size).clamp(inner.x, inner.x + inner.width),
            (origin.y + caret.y_em * view.size).clamp(inner.y, inner.y + inner.height),
            1.5, (caret.height_em * view.size).min(inner.height)))
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &mut self,
        renderer: &mut Renderer,
        layer: &mut Layer,
        rect: Rect,
        size: f32,
        focused: bool,
        secret: bool,
        placeholder: &str,
        framed: bool,
    ) {
        let edge = if focused { size / 8. } else { size / 16. };
        let radius = size / 4.;
        if framed {
            layer.rounded_rect(
                rect,
                radius,
                color(if focused { 0x67d4ff } else { 0x526170 }),
            );
            layer.rounded_rect(
                Rect::new(
                    rect.x + edge,
                    rect.y + edge,
                    rect.width - 2. * edge,
                    rect.height - 2. * edge,
                ),
                (radius - edge).max(0.),
                color(if self.single_line { 0x36343b } else { 0x0e141b }),
            );
        }
        let view = View { rect, size, secret };
        if self.view.is_none_or(|old| old.rect.width != rect.width || old.rect.height != rect.height || old.size != size || old.secret != secret) {
            self.follow_caret = true;
        }
        self.view = Some(view);
        self.visible = true;
        let inner = view.inner();
        if self.value.is_empty() && self.composition.is_none() {
            let h = size * 1.3;
            let placeholder_rect = if self.single_line || self.center_one_line {
                Rect::new(inner.x, inner.y + (inner.height - h) / 2., inner.width, h)
            } else {
                inner
            };
            renderer.label(
                layer,
                placeholder,
                placeholder_rect,
                size,
                color(0xb7c2ce),
                false,
            );
        }
        if let Some(block) = self.prepare_view(&mut renderer.text, renderer.faces.prose[0], true) {
            let layout = renderer.text.measure(block);
            let caret = layout.caret_rect(self.placed(layout, secret));
            let origin = self.origin(layout, view);
            if focused {
                let selection = if let Some(c) = &self.composition {
                    // Underline composition separately; committed selection is being replaced.
                    c.replace.start..c.replace.start
                } else { self.range() };
                for span in layout.selection(self.display_byte(selection.start, secret)..self.display_byte(selection.end, secret)) {
                    layer.clipped_rect(
                        Rect::new(
                            origin.x + span.x_em * size,
                            origin.y + span.y_em * size,
                            span.width_em * size,
                            span.height_em * size,
                        ),
                        color(0x245773),
                        inner,
                    );
                }
            }
            if let Some(c) = &self.composition {
                let (display, _) = self.display();
                let map = |byte| if secret { display[..byte].graphemes(true).count() * "•".len() } else { byte };
                for span in layout.selection(map(c.replace.start)..map(c.replace.start + c.text.len())) {
                    layer.clipped_rect(Rect::new(origin.x + span.x_em * size,
                        origin.y + (span.y_em + span.height_em) * size - 2.,
                        span.width_em * size, 1.), color(0x67d4ff), inner);
                }
            }
            layer.draws.push(Draw {
                block,
                at: origin,
                size,
                color: color(0xe5eaf0),
                clip: Some(inner),
                ..Default::default()
            });
            if focused && self.composition.as_ref().is_none_or(|c| c.cursor.is_some()) {
                layer.clipped_rect(
                    Rect::new(
                        origin.x + caret.x_em * size,
                        origin.y + caret.y_em * size,
                        1.5,
                        caret.height_em * size,
                    ),
                    color(0x67d4ff),
                    inner,
                );
            }
        }
    }
}

fn normalize(value: &str, single_line: bool) -> String {
    if single_line { value.replace(['\r', '\n'], "") }
    else { value.replace("\r\n", "\n").replace('\r', "\n") }
}
fn char_boundary(value: &str, byte: usize) -> usize {
    let mut byte = byte.min(value.len());
    while !value.is_char_boundary(byte) { byte -= 1; }
    byte
}
fn reveal(offset: f32, at: f32, extent: f32, visible: f32) -> f32 {
    if at < offset || extent >= visible { at }
    else if at + extent > offset + visible { at + extent - visible }
    else { offset }
}

// Word policy belongs to Tau, not shaping. Skip whitespace, then cross a run of
// Unicode alphanumeric/underscore graphemes or punctuation/emoji graphemes.
// Masked fields act as one word rather than exposing the secret's word structure.
struct Words<'a> { text: &'a str, secret_len: Option<usize> }
fn word_kind(grapheme: &str) -> u8 {
    if grapheme.chars().all(char::is_whitespace) { 0 }
    else if grapheme.chars().any(|c| c.is_alphanumeric() || c == '_') { 1 }
    else { 2 }
}
impl Boundaries for Words<'_> {
    fn prev_word(&self, byte: usize) -> Option<usize> {
        if self.secret_len.is_some() { return Some(0); }
        let mut result = byte;
        let mut kind = 0;
        for (at, g) in self.text[..byte].grapheme_indices(true).rev() {
            let next = word_kind(g);
            if kind != 0 && next != kind { break; }
            result = at;
            kind = next;
        }
        Some(result)
    }
    fn next_word(&self, byte: usize) -> Option<usize> {
        if let Some(len) = self.secret_len { return Some(len); }
        let mut result = byte;
        let mut kind = 0;
        for (at, g) in self.text[byte..].grapheme_indices(true) {
            let next = word_kind(g);
            if kind != 0 && next != kind { break; }
            result = byte + at + g.len();
            kind = next;
        }
        Some(result)
    }
}

#[cfg(test)]
mod tests;
