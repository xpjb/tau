use crate::render::{Layer, Renderer, color};
use sanscale::{Align, Draw, Rect, Style, Vec2};
use unicode_segmentation::UnicodeSegmentation;

#[derive(Default)]
pub struct Editor {
    pub value: String,
    pub cursor: usize,
    pub anchor: usize,
    pub preedit: String,
    pub single_line: bool,
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
    pub fn line(value: String) -> Self {
        Self {
            single_line: true,
            ..Self::new(value.replace(['\r', '\n'], ""))
        }
    }
    pub fn range(&self) -> std::ops::Range<usize> {
        self.cursor.min(self.anchor)..self.cursor.max(self.anchor)
    }
    pub fn selected(&self) -> &str {
        &self.value[self.range()]
    }
    pub fn replace(&mut self, value: &str) {
        let value = if self.single_line {
            value.replace(['\r', '\n'], "")
        } else {
            value.replace("\r\n", "\n").replace('\r', "\n")
        };
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
    fn display(&self, secret: bool) -> (String, usize, usize) {
        if secret {
            (
                "•".repeat(self.value.chars().count()),
                self.value[..self.cursor].chars().count() * "•".len(),
                self.value[..self.anchor].chars().count() * "•".len(),
            )
        } else {
            (
                format!(
                    "{}{}{}",
                    &self.value[..self.cursor],
                    self.preedit,
                    &self.value[self.cursor..]
                ),
                self.cursor,
                self.anchor,
            )
        }
    }
    fn style(&self, renderer: &Renderer, inner: Rect, size: f32) -> Style {
        Style {
            chain: renderer.faces.prose[0],
            wrap_em: (!self.single_line).then_some(inner.width / size),
            align: Align::Left,
            line_spacing: 1.2,
        }
    }
    fn inner(&self, rect: Rect, size: f32) -> Rect {
        let pad = size * 0.75;
        Rect::new(
            rect.x + pad,
            rect.y + pad,
            (rect.width - pad * 2.).max(1.),
            (rect.height - pad * 2.).max(1.),
        )
    }
    fn text_origin(&self, inner: Rect, size: f32, height: f32, caret: sanscale::Rect) -> Vec2 {
        if self.single_line {
            Vec2::new(
                inner.x - (caret.x * size - inner.width + 2.).max(0.),
                inner.y + (inner.height - height * size) / 2.,
            )
        } else {
            Vec2::new(
                inner.x,
                inner.y - (caret.y * size + caret.height * size - inner.height).max(0.),
            )
        }
    }
    pub fn hit(
        &mut self,
        renderer: &mut Renderer,
        rect: Rect,
        point: Vec2,
        size: f32,
        secret: bool,
        extend: bool,
    ) {
        let inner = self.inner(rect, size);
        let (display, cursor, _) = self.display(secret);
        if let Some(block) = renderer
            .text
            .shape_transient(&display, &self.style(renderer, inner, size))
        {
            let layout = renderer.text.measure(block);
            let caret = layout.caret_rect(cursor);
            let origin = self.text_origin(
                inner,
                size,
                layout.height_em(),
                Rect::new(caret.x_em, caret.y_em, 0., caret.height_em),
            );
            if let Some(hit) = layout.hit_test(Vec2::new(
                (point.x - origin.x) / size,
                (point.y - origin.y) / size,
            )) {
                let mut byte = if secret {
                    self.value
                        .char_indices()
                        .nth(hit.byte_index / "•".len())
                        .map_or(self.value.len(), |(b, _)| b)
                } else {
                    hit.byte_index.min(self.value.len())
                };
                while !self.value.is_char_boundary(byte) {
                    byte -= 1;
                }
                self.move_to(byte, extend);
            }
        }
    }
    pub fn height(&self, renderer: &mut Renderer, width: f32, size: f32) -> f32 {
        let inner = self.inner(Rect::new(0., 0., width, 56.), size);
        let display = self.display(false).0;
        let height = renderer
            .text
            .shape_transient(&display, &self.style(renderer, inner, size))
            .map_or(0., |block| renderer.text.measure(block).height_em() * size);
        (height + size * 1.5).clamp(size * 3.5, size * 10.)
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
        let inner = self.inner(rect, size);
        let (display, cursor, anchor) = self.display(secret);
        let style = self.style(renderer, inner, size);
        if display.is_empty() {
            let h = size * 1.3;
            let placeholder_rect = if self.single_line {
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
        if let Some(block) = renderer.text.shape_transient(&display, &style) {
            let layout = renderer.text.measure(block);
            let caret = layout.caret_rect(cursor);
            let origin = self.text_origin(
                inner,
                size,
                layout.height_em(),
                Rect::new(caret.x_em, caret.y_em, 0., caret.height_em),
            );
            if focused {
                for span in layout.selection(cursor.min(anchor)..cursor.max(anchor)) {
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
            layer.draws.push(Draw {
                block,
                at: origin,
                size,
                color: color(0xe5eaf0),
                clip: Some(inner),
                ..Default::default()
            });
            if focused {
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
