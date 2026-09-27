//! Small, literal rich labels. Provider strings are never parsed as markup.
use super::{Content, INK};
use sanscale::{Align, BlockKey, FontSpan, PaintHandle, PaintSpan, ParagraphKey, ParagraphSource, ShapedHandle, Style, TextService};
use std::{borrow::Cow, ops::Range};
use tau_markdown::Faces;
use unicode_segmentation::UnicodeSegmentation;

pub(crate) struct RichLabel {
    namespace: u32,
    generation: u32,
    content: Content,
    fonts: Vec<FontSpan>,
    pub block: ShapedHandle,
    pub paint: Option<PaintHandle>,
}
impl RichLabel {
    pub fn new(namespace: u32) -> Self {
        Self { namespace, generation: 0, content: Content::default(), fonts: vec![], block: ShapedHandle::INVALID, paint: None }
    }
    pub fn layout(&mut self, text: &mut TextService, faces: Faces, content: &Content, width: f32, size: f32) -> f32 {
        if self.content != *content {
            self.generation = self.generation.checked_add(1).expect("tooltip text generation");
            self.fonts.clear();
            // Font boundaries must be grapheme-safe, including arbitrary error text.
            let mut spans = content.spans.iter().peekable();
            for (at, g) in content.text.grapheme_indices(true) {
                while spans.peek().is_some_and(|s| s.range.end <= at) { spans.next(); }
                if spans.peek().is_some_and(|s| s.bold && s.range.contains(&at)) {
                    if let Some(last) = self.fonts.last_mut().filter(|s| s.range.end == at) {
                        last.range.end = at + g.len();
                    } else {
                        self.fonts.push(FontSpan { range: at..at + g.len(), chain: faces.prose[1] });
                    }
                }
            }
            let colors: Vec<_> = content.spans.iter().filter(|s| s.tint != INK)
                .map(|s| PaintSpan { range: s.range.clone(), color: crate::render::color(s.tint) }).collect();
            let paint = (!colors.is_empty()).then(|| text.register_paint(&colors).expect("tooltip paint"));
            if let Some(old) = self.paint.take() { text.drop_paint(old); }
            self.paint = paint;
            self.content = content.clone();
        }
        let mut start = 0;
        let parts: Vec<_> = content.text.split('\n').map(|s| {
            let range = start..start + s.len(); start += s.len() + 1; range
        }).collect();
        let keys: Vec<_> = (0..parts.len()).map(|i| ParagraphKey {
            namespace: u64::from(self.namespace), slot: i as u32, generation: self.generation,
        }).collect();
        let source = Source { content: &content.text, fonts: &self.fonts, parts };
        let style = Style { chain: faces.prose[0], wrap_em: Some(width.max(1.) / size), align: Align::Left, line_spacing: 1.2 };
        self.block = text.shape(BlockKey(u64::from(self.namespace) << 32), &style, &keys, &source)
            .unwrap_or(ShapedHandle::INVALID);
        text.measure(self.block).height_em() * size
    }
}
struct Source<'a> {
    content: &'a str,
    fonts: &'a [FontSpan],
    parts: Vec<Range<usize>>,
}
impl ParagraphSource for Source<'_> {
    fn paragraph_text(&self, i: usize, _: ParagraphKey) -> Option<Cow<'_, str>> {
        self.parts.get(i).map(|r| Cow::Borrowed(&self.content[r.clone()]))
    }
    fn paragraph_fonts(&self, i: usize, _: ParagraphKey) -> Cow<'_, [FontSpan]> {
        let r = &self.parts[i];
        Cow::Owned(self.fonts.iter().filter_map(|s| {
            let start = s.range.start.max(r.start);
            let end = s.range.end.min(r.end);
            (start < end).then(|| FontSpan { range: start - r.start..end - r.start, chain: s.chain })
        }).collect())
    }
}
