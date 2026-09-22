use crate::render::{Layer, Renderer, color};
use sanscale::{Align, Draw, Rect, Style, Vec2};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct Editor {
    pub value: String,
    pub cursor: usize,
    pub anchor: usize,
    pub preedit: String,
    undo: Vec<(String, usize, usize)>,
    redo: Vec<(String, usize, usize)>,
}
impl Editor {
    pub fn new(value: String) -> Self {
        let cursor = value.len();
        Self {
            value,
            cursor,
            anchor: cursor,
            ..Default::default()
        }
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }
    pub fn selected(&self) -> &str {
        &self.value[self.range()]
    }
    pub fn replace(&mut self, value: &str) {
        let value = value.replace("\r\n", "\n").replace('\r', "\n");
        if self.value.len() - self.range().len() + value.len() > tau_protocol::MAX_REQUEST_BYTES {
            return;
        }
        self.undo
            .push((self.value.clone(), self.cursor, self.anchor));
        if self.undo.len() > 64 {
            self.undo.remove(0);
        }
        self.redo.clear();
        let range = self.range();
        self.cursor = range.start + value.len();
        self.anchor = self.cursor;
        self.value.replace_range(range, &value);
        self.preedit.clear();
    }
    pub fn previous(&self) -> usize {
        self.value[..self.cursor]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(i, _)| i)
    }
    pub fn next(&self) -> usize {
        self.value[self.cursor..]
            .graphemes(true)
            .next()
            .map_or(self.value.len(), |s| self.cursor + s.len())
    }
    pub fn move_to(&mut self, cursor: usize, shift: bool) {
        self.cursor = cursor;
        if !shift {
            self.anchor = cursor;
        }
    }
    pub fn backspace(&mut self) {
        if self.range().is_empty() {
            self.anchor = self.previous();
        }
        self.replace("");
    }
    pub fn delete(&mut self) {
        if self.range().is_empty() {
            self.anchor = self.next();
        }
        self.replace("");
    }
    pub fn undo(&mut self, redo: bool) {
        let (from, to) = if redo {
            (&mut self.redo, &mut self.undo)
        } else {
            (&mut self.undo, &mut self.redo)
        };
        if let Some((value, cursor, anchor)) = from.pop() {
            to.push((
                std::mem::replace(&mut self.value, value),
                self.cursor,
                self.anchor,
            ));
            self.cursor = cursor;
            self.anchor = anchor;
        }
    }
    #[allow(clippy::too_many_arguments)]
    pub fn draw(
        &self,
        renderer: &mut Renderer,
        layer: &mut Layer,
        rect: Rect,
        size: f32,
        focused: bool,
        secret: bool,
        placeholder: &str,
    ) {
        layer.rect(rect, color(if focused { 0x1c2936 } else { 0x18212b }));
        let inner = Rect::new(
            rect.x + 10.,
            rect.y + 8.,
            (rect.width - 20.).max(1.),
            (rect.height - 16.).max(1.),
        );
        let display = if secret {
            "•".repeat(self.value.chars().count())
        } else {
            format!(
                "{}{}{}",
                &self.value[..self.cursor],
                self.preedit,
                &self.value[self.cursor..]
            )
        };
        if display.is_empty() {
            renderer.label(layer, placeholder, inner, size, color(0x82909f), false);
        }
        let style = Style {
            chain: renderer.faces.prose[0],
            wrap_em: Some(inner.width / size),
            align: Align::Left,
            line_spacing: 1.15,
        };
        if let Some(block) = renderer.text.shape_transient(&display, &style) {
            let layout = renderer.text.measure(block);
            let cursor = if secret {
                self.value[..self.cursor].chars().count() * "•".len()
            } else {
                self.cursor
            };
            let caret = layout.caret_rect(cursor);
            let scroll = (caret.y_em * size + caret.height_em * size - inner.height).max(0.);
            if focused && !secret {
                for span in layout.selection(self.range()) {
                    layer.clipped_rect(
                        Rect::new(
                            inner.x + span.x_em * size,
                            inner.y + span.y_em * size - scroll,
                            span.width_em * size,
                            span.height_em * size,
                        ),
                        color(0x245773),
                        inner,
                    );
                }
            }
            layer.draws.push(Draw {
                block,
                at: Vec2::new(inner.x, inner.y - scroll),
                size,
                color: color(0xe5eaf0),
                clip: Some(inner),
                ..Default::default()
            });
            if focused {
                layer.clipped_rect(
                    Rect::new(
                        inner.x + caret.x_em * size,
                        inner.y + caret.y_em * size - scroll,
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
