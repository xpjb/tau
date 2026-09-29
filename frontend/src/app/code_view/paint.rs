use super::*;
pub(super) fn document(doc: &Document, selection: Option<std::ops::Range<usize>>, paints: &mut HashMap<u64, (Vec<PaintSpan>, Option<PaintHandle>)>, scroll: &mut ScrollState, horizontal: &mut ScrollState, viewport: Rect, layer: &mut Layer, cx: &mut Context<'_>) -> (f32, f32) {
    let s = cx.ui.scale;
            let line_height = if cx.ui.mobile { 28. } else { 24. } * s;
            let gutter = (44. + doc.lines.len().to_string().len().saturating_sub(3) as f32 * 8.) * s;
            scroll.max = (doc.lines.len() as f32 * line_height + 16. * s - viewport.height).max(0.);
            scroll.value = scroll.value.clamp(0., scroll.max);
            let first = (scroll.value / line_height).floor() as usize;
            let end = (first + (viewport.height / line_height).ceil() as usize + 2).min(doc.lines.len());
            let mut visible = HashSet::new();
            let mut widest = viewport.width;
            layer.rect(Rect::new(viewport.x, viewport.y, gutter, viewport.height), color(0x111923));
            for i in first..end {
                let line = &doc.lines[i];
                visible.insert(line.id);
                let y = viewport.y + i as f32 * line_height - scroll.value;
                if selection.as_ref().is_some_and(|r| r.contains(&i)) {
                    layer.clipped_rect(
                        Rect::new(viewport.x, y, viewport.width, line_height),
                        color(0x213c57),
                        viewport,
                    );
                }
                cx.services.renderer.clipped_label(
                    layer,
                    &(i + 1).to_string(),
                    Rect::new(viewport.x + 8. * s, y + 5. * s, gutter - 14. * s, line_height),
                    12. * s,
                    color(if selection.as_ref().is_some_and(|r| r.contains(&i)) { 0x8bd6ff } else { 0x6c7d90 }),
                    false,
                    viewport,
                );
                // Cap a single pathological/minified line's shaping work while
                // retaining its original full text for selection and copying.
                let mut stop = line.text.len().min(16 * 1024);
                while !line.text.is_char_boundary(stop) {
                    stop -= 1;
                }
                let mut text = line.text[..stop].replace('\t', "    ");
                if stop < line.text.len() {
                    text.push_str(" … [long line preview limited; Copy keeps full text]");
                }
                let tabs = line.text[..stop].contains('\t');
                let byte = |n: usize| {
                    let n = n.min(stop);
                    n + if tabs { line.text[..n].bytes().filter(|&b| b == b'\t').count() * 3 } else { 0 }
                };
                let spans = line
                    .paint
                    .iter()
                    .filter(|p| p.range.start < stop)
                    .map(|p| PaintSpan { range: byte(p.range.start)..byte(p.range.end), color: color(p.color) })
                    .collect::<Vec<_>>();
                let cached = paints.entry(line.id).or_insert_with(|| (vec![], None));
                if cached.0 != spans {
                    if let Some(p) = cached.1.take() {
                        cx.services.renderer.text.drop_paint(p);
                    }
                    cached.1 =
                        if spans.is_empty() { None } else { cx.services.renderer.text.register_paint(&spans).ok() };
                    cached.0 = spans;
                }
                let namespace = 0x5441_5546_0000_0000 | doc.namespace;
                let key = ParagraphKey { namespace, slot: line.id as u32, generation: 1 };
                let style = Style {
                    chain: cx.services.renderer.faces.mono[0],
                    wrap_em: None,
                    align: Align::Left,
                    line_spacing: 1.2,
                };
                let key_id = 0xe000_0000_0000_0000 | (doc.namespace << 32) | line.id;
                if let Some(block) = cx.services.renderer.text.shape(BlockKey(key_id), &style, &[key], &Source(&text)) {
                    let size = 14. * s;
                    widest =
                        widest.max(cx.services.renderer.text.measure(block).width_em() * size + gutter + 24. * s);
                    layer.draws.push(Draw {
                        block,
                        at: Vec2::new(viewport.x + gutter + 8. * s - horizontal.value, y + 4. * s),
                        size,
                        color: color(0xd8dee9),
                        clip: Some(Rect::new(
                            viewport.x + gutter,
                            viewport.y,
                            viewport.width - gutter,
                            viewport.height,
                        )),
                        paint: cached.1,
                    });
                }
            }
            horizontal.max = (widest - viewport.width).max(horizontal.value);
            paints.retain(|id, (_, paint)| {
                let keep = visible.contains(id);
                if !keep && let Some(p) = paint.take() {
                    cx.services.renderer.text.drop_paint(p);
                }
                keep
            });
    (line_height, gutter)
}

pub(super) fn matched_label(code: &mut View, row: usize, name: &str, rect: Rect, viewport: Rect, layer: &mut Layer, cx: &mut Context<'_>) {
    let spans = code.highlighter.highlights(name).into_iter().map(|range| PaintSpan { range, color: color(0x67d4ff) }).collect::<Vec<_>>();
    let cached = code.paints.entry(row as u64).or_insert_with(|| (vec![], None));
    if cached.0 != spans {
        if let Some(paint) = cached.1.take() { cx.services.renderer.text.drop_paint(paint); }
        cached.1 = if spans.is_empty() { None } else { cx.services.renderer.text.register_paint(&spans).ok() };
        cached.0 = spans;
    }
    let style = Style { chain: cx.services.renderer.faces.mono[0], wrap_em: None, align: Align::Left, line_spacing: 1. };
    if let Some(block) = cx.services.renderer.text.shape_transient(name, &style) {
        layer.draws.push(Draw { block, at: Vec2::new(rect.x, rect.y), size: 14.*cx.ui.scale, color: color(0xd8dee9), clip: Some(crate::render::intersect(rect, viewport)), paint: cached.1 });
    }
}
