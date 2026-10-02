use super::*;

#[test]
fn link_underlines_stay_below_each_measured_baseline() {
    let (mut text, faces) = setup();
    for source in [
        "See [https://example.org with **bold**, _italic_ and `code` labels that wrap](https://example.org) here.",
        "# [A heading link that wraps](https://example.org)",
        "[first line\nsecond line](https://example.org)",
        "> [A quoted link that wraps](https://example.org)",
        "| Link |\n| --- |\n| [A table link that wraps](https://example.org) |",
    ] {
        let doc = Document::new(source);
        for size in [12., 16., 28.] {
            let theme = Theme::default();
            let mut view = Preview::new(902);
            view.sync(&doc, &mut text, faces, theme, 130., size);
            let scene = view.scene(
                &mut text,
                &doc,
                Rect::new(30., 60., 130., 1600.),
                Vec2::new(7., 11.),
            );
            assert!(
                scene.over.len() > 1,
                "fixture must exercise multiple lines/runs"
            );
            let mut underlines = scene.over.iter();
            for &(id, at) in &scene.placed {
                let c = &view.texts[&id];
                let layout = text.measure(c.handle);
                for run in &c.rich.runs {
                    if run.flags & inline::LINK == 0 {
                        continue;
                    }
                    for span in layout.selection(run.range.clone()) {
                        let line = layout.line(span.line).unwrap();
                        let baseline = at.y + line.baseline_em * c.size;
                        let underline = underlines.next().unwrap();
                        let gap_em = (underline.rect.y - baseline) / c.size;
                        assert!(
                            (0.04..0.2).contains(&gap_em),
                            "underline must sit just below baseline, not across glyphs: gap={gap_em}em, size={size}, source={source:?}"
                        );
                        assert!(
                            underline.rect.y + underline.rect.height
                                <= at.y + (line.top_em + line.height_em) * c.size
                        );
                        assert!((underline.rect.x - (at.x + span.x_em * c.size)).abs() < 0.001);
                        assert!((underline.rect.width - span.width_em * c.size).abs() < 0.001);
                        assert_eq!(underline.rect.height, 1.);
                        assert_eq!(underline.color, theme.accent());
                    }
                }
            }
            assert!(
                underlines.next().is_none(),
                "only link spans are underlined"
            );
            view.release(&mut text);
        }
    }
}

#[test]
fn links_and_copy_follow_projected_text_even_when_only_destination_changes() {
    let (mut text, faces) = setup();
    let mut doc = Document::new("**Read** [Docs](https://example.org/old) &amp; café");
    let mut view = Preview::new(900);
    view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
    assert_eq!(
        view.copy_selection(&doc, 0..doc.source().len_bytes()),
        "Read Docs & café"
    );
    let id = doc.blocks()[0].elements().next().unwrap().id;
    let scene = view.scene(
        &mut text,
        &doc,
        Rect::new(0., 0., 500., 200.),
        Vec2::new(0., 0.),
    );
    let at = scene
        .placed
        .iter()
        .find(|(placed, _)| *placed == id)
        .unwrap()
        .1;
    let c = &view.texts[&id];
    let layout = text.measure(c.handle);
    let caret = layout.caret_rect(layout.caret_at("Read D".len()));
    let point = Vec2::new(
        at.x + caret.x_em * c.size,
        at.y + (caret.y_em + caret.height_em * 0.5) * c.size,
    );
    assert_eq!(
        view.hit_link(&scene, point, &text).as_deref(),
        Some("https://example.org/old")
    );
    assert!(
        !view
            .selection(&scene, &text, &doc, 0..doc.source().len_bytes())
            .is_empty()
    );
    let source = doc.source().to_string();
    let start = source.find("/old").unwrap();
    doc.edit(start..start + 4, "/new").unwrap();
    view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
    let scene = view.scene(
        &mut text,
        &doc,
        Rect::new(0., 0., 500., 200.),
        Vec2::new(0., 0.),
    );
    assert_eq!(
        view.hit_link(&scene, point, &text).as_deref(),
        Some("https://example.org/new")
    );
    assert_eq!(
        view.copy_selection(&doc, 0..doc.source().len_bytes()),
        "Read Docs & café"
    );
}

#[test]
fn soft_breaks_stream_and_copy_as_real_lines() {
    let (mut text, faces) = setup();
    for source in ["first\nsecond", "- first\n  second", "> first\n> second"] {
        let mut doc = Document::default();
        let mut view = Preview::new(901);
        for chunk in source.split_inclusive('\n') {
            doc.append(chunk).unwrap();
            view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
        }
        assert_eq!(doc.source().to_string(), source);
        assert_eq!(doc.blocks().len(), 1);
        let c = &view.texts[&doc.blocks()[0].id];
        assert_eq!(c.rich.text, "first\nsecond");
        assert_eq!(text.measure(c.handle).line_count(), 2);
        assert_eq!(
            view.copy_selection(&doc, 0..doc.source().len_bytes()),
            "first\nsecond"
        );
        doc.append("\n\nthird").unwrap();
        view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
        assert_eq!(doc.blocks().len(), 2, "blank lines still split paragraphs");
        view.release(&mut text);
    }
}

#[test]
fn heights_append_update_splice_and_search() {
    let mut h = Heights::from(vec![10., 20., 30.]);
    assert_eq!(h.total(), 60.);
    assert_eq!(h.row_at(10.), 1);
    h.push(40.);
    assert_eq!(h.prefix(4), 100.);
    h.set(1, 25.);
    assert_eq!(h.total(), 105.);
    h.splice(1..2, 2);
    assert_eq!(h.values, vec![10., 0., 0., 30., 40.]);
    h.set(1, 4.);
    h.set(2, 6.);
    assert_eq!(h.total(), 90.);
}
fn setup() -> (TextService, Faces) {
    let mut text = TextService::new();
    let bytes: [&'static [u8]; 4] = [
        include_bytes!("../../../frontend/assets/DejaVuSans.ttf"),
        include_bytes!("../../../frontend/assets/DejaVuSans-Bold.ttf"),
        include_bytes!("../../../frontend/assets/DejaVuSans-Oblique.ttf"),
        include_bytes!("../../../frontend/assets/DejaVuSans-BoldOblique.ttf"),
    ];
    let prose = bytes.map(|bytes| {
        let f = text.map_font(Arc::new(bytes), 0).unwrap();
        text.register_chain(&[f]).expect("font chain capacity")
    });
    let faces = Faces { prose, mono: prose };
    (text, faces)
}
#[test]
fn color_theme_never_parses_or_shapes_and_tables_update_one_row() {
    let (mut text, faces) = setup();
    let mut doc = Document::new(
        "# Hi\n\n**bold _both_** and `code` &amp; [link](url)\n\n| A | B |\n| --- | ---: |\n| x | **y** |\n",
    );
    let mut view = Preview::new(300);
    view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
    let revision = doc.revision();
    #[cfg(feature = "perf-counters")]
    sanscale::profiling::reset_work_counters();
    let theme = Theme {
        alternate: true,
        ..Default::default()
    };
    let w = view.sync(&doc, &mut text, faces, theme, 500., 17.);
    assert_eq!(w.layout_requests, 0);
    assert_eq!(doc.revision(), revision);
    #[cfg(feature = "perf-counters")]
    {
        let c = sanscale::profiling::work_counters();
        assert_eq!((c.shape_calls, c.flow_calls, c.source_reads), (0, 0, 0));
    }
    doc.append("| new | a much longer streamed cell that wraps over several lines without changing column widths |\n").unwrap();
    let w = view.sync(&doc, &mut text, faces, theme, 500., 17.);
    assert_eq!(w.measured_rows, 1);
    assert_eq!(w.layout_requests, 2);
    assert!(w.indexed_rows <= 1);
    view.release(&mut text);
}
#[test]
fn missed_history_and_multiple_edits_reconcile_without_losing_rows() {
    let (mut text, faces) = setup();
    let mut doc = Document::new("| A | B |\n| --- | --- |\n");
    let mut view = Preview::new(301);
    view.sync(&doc, &mut text, faces, Theme::default(), 400., 17.);
    for _ in 0..4 {
        doc.append("| x | y |\n").unwrap();
    }
    view.sync(&doc, &mut text, faces, Theme::default(), 400., 17.);
    for _ in 0..70 {
        doc.append("| xx | yy |\n").unwrap();
    }
    view.sync(&doc, &mut text, faces, Theme::default(), 400., 17.);
    let scene = view.scene(
        &mut text,
        &doc,
        Rect::new(0., 0., 400., 200.),
        Vec2::new(0., 0.),
    );
    assert!(!scene.draws.is_empty());
    assert!(scene.draws.len() < 20);
    view.release(&mut text);
}
#[test]
fn changed_row_height_moves_following_blocks_without_reshaping_them() {
    let (mut text, faces) = setup();
    let mut doc =
        Document::new("| A | B |\n| --- | --- |\n| short | value |\n\nA following paragraph.");
    let mut view = Preview::new(302);
    view.sync(&doc, &mut text, faces, Theme::default(), 320., 17.);
    let after = doc.blocks()[1].id;
    let old_y = view.block_y(after).unwrap();
    let old_handle = view.texts[&after].handle;
    let at = doc.source().to_string().find("value").unwrap();
    doc.edit(
        at..at + 5,
        "a much longer cell that takes several visual lines to display",
    )
    .unwrap();
    let w = view.sync(&doc, &mut text, faces, Theme::default(), 320., 17.);
    assert_eq!(w.layout_requests, 1);
    assert_eq!(w.measured_rows, 1);
    assert!(view.block_y(after).unwrap() > old_y);
    assert_eq!(view.texts[&after].handle, old_handle);
    view.release(&mut text);
}
#[test]
fn projected_clicks_map_entities_and_shifted_cells_back_to_source() {
    let (mut text, faces) = setup();
    let mut doc = Document::new("| A | B |\n| --- | --- |\n| x | &amp; café |\n");
    let mut view = Preview::new(303);
    for value in ["x", "much longer"] {
        if value != "x" {
            let i = doc.source().to_string().find("| x |").unwrap() + 2;
            doc.edit(i..i + 1, value).unwrap();
        }
        view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
        let scene = view.scene(
            &mut text,
            &doc,
            Rect::new(10., 20., 500., 300.),
            Vec2::new(0., 0.),
        );
        let Content::Table(t) = &doc.blocks()[0].content else {
            panic!()
        };
        let id = t.rows[1].cells[1].id;
        let c = &view.texts[&id];
        let at = scene.placed.iter().find(|p| p.0 == id).unwrap().1;
        let layout = text.measure(c.handle);
        let caret = layout.caret_rect(layout.caret_at(0));
        let point = Vec2::new(
            at.x + caret.x_em * c.size,
            at.y + (caret.y_em + caret.height_em * 0.5) * c.size,
        );
        assert_eq!(
            view.hit_source(&scene, point, &text, &doc),
            doc.source().to_string().find("&amp;")
        );
    }
    view.release(&mut text);
}
#[test]
fn reusing_a_view_for_another_document_cannot_alias_old_keys() {
    let (mut text, faces) = setup();
    let a = Document::new("old **content**");
    let b = Document::new("new _different words_");
    let mut view = Preview::new(304);
    view.sync(&a, &mut text, faces, Theme::default(), 400., 17.);
    view.sync(&b, &mut text, faces, Theme::default(), 400., 17.);
    let c = &view.texts[&b.blocks()[0].id];
    assert_eq!(
        text.measure(c.handle).len_bytes(),
        "new different words".len()
    );
    view.release(&mut text);
}
#[test]
fn edited_layout_matches_a_fresh_view_including_table_structure() {
    let (mut text, faces) = setup();
    let mut doc = Document::new(
        "# Header\n\n| A | B |\n| --- | --- |\n| first | **second** |\n| third | fourth |\n\nAfter the table.\n",
    );
    let mut view = Preview::new(305);
    let mut seed = 73u64;
    for step in 0..100 {
        view.sync(&doc, &mut text, faces, Theme::default(), 380., 17.);
        let mut cold = Preview::new(10000 + step);
        cold.sync(&doc, &mut text, faces, Theme::default(), 380., 17.);
        assert!((view.height - cold.height).abs() < 0.02);
        for block in doc.blocks() {
            let a = &view.blocks[&block.id];
            let b = &cold.blocks[&block.id];
            assert!((a.y - b.y).abs() < 0.02);
            assert!((a.height - b.height).abs() < 0.02);
        }
        cold.release(&mut text);
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
        let s = doc.source().to_string();
        let points = s
            .char_indices()
            .map(|(i, _)| i)
            .chain([s.len()])
            .collect::<Vec<_>>();
        let a = seed as usize % points.len();
        let z = (a + ((seed >> 24) as usize) % 4).min(points.len() - 1);
        doc.edit(
            points[a]..points[z],
            [" | ", "\n", "**", "é", "", ":---:"][(seed >> 32) as usize % 6],
        )
        .unwrap();
    }
    view.release(&mut text);
}
#[test]
fn combined_faces_grapheme_boundaries_and_real_hard_breaks() {
    let (mut text, faces) = setup();
    let doc = Document::new("***both*** **a**\u{301}  \nnext");
    let mut view = Preview::new(306);
    view.sync(&doc, &mut text, faces, Theme::default(), 500., 17.);
    let c = &view.texts[&doc.blocks()[0].id];
    assert_eq!(c.rich.text, "both a\u{301}\nnext");
    assert_eq!(
        c.fonts,
        vec![
            FontSpan {
                range: 0..4,
                chain: faces.prose[3]
            },
            FontSpan {
                range: 5..8,
                chain: faces.prose[1]
            }
        ]
    );
    assert_eq!(text.measure(c.handle).line_count(), 2);
    assert_eq!(text.measure(c.handle).len_bytes(), c.rich.text.len());
    let w = view.sync(
        &doc,
        &mut text,
        faces,
        Theme {
            italic: false,
            ..Default::default()
        },
        500.,
        17.,
    );
    assert_eq!(w.layout_requests, 1);
    assert_eq!(
        view.texts[&doc.blocks()[0].id].fonts[0].chain,
        faces.prose[1]
    );
    view.release(&mut text);
}
