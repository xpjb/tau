//! Real font/layout/component regressions, not a second synthetic text engine.
use super::*;
use std::sync::Arc;

struct Fixture { text: TextService, chain: FontChainHandle }
impl Fixture {
    fn new() -> Self {
        let mut text = TextService::new();
        let font = text.map_font(Arc::new(include_bytes!("../../assets/DejaVuSansMono.ttf").as_slice()), 0).unwrap();
        let chain = text.register_chain(&[font]).unwrap();
        Self { text, chain }
    }
    fn view(&mut self, editor: &mut Editor, width_em: f32, height_em: f32, secret: bool) -> ShapedHandle {
        editor.view = Some(View { rect: Rect::new(0., 0., (width_em + 1.5) * 16., (height_em + 1.5) * 16.), size: 16., secret });
        editor.visible = true;
        editor.prepare_view(&mut self.text, self.chain, true).unwrap()
    }
    fn key(&mut self, editor: &mut Editor, key: &str, ctrl: bool, shift: bool) -> bool {
        editor.key(&mut self.text, self.chain, key, ctrl, shift)
    }
    fn layout<'a>(&'a self, editor: &Editor) -> &'a Layout { self.text.measure(editor.layout.as_ref().unwrap().block) }
}

#[test]
fn composer_centers_one_visual_line_without_changing_multiline_editors() {
    let mut f = Fixture::new();
    let mut e = Editor::composer("hello".into());
    f.view(&mut e, 30., 2., false); // 56px field, matching the composer minimum.
    let view = e.view.unwrap();
    let inner = view.inner();
    let layout = f.layout(&e);
    assert_eq!(layout.line_count(), 1);
    let origin = e.origin(layout, view);
    assert!((origin.y + layout.height_em() * view.size / 2. - (inner.y + inner.height / 2.)).abs() < 0.01);
    let caret = e.ime_rect(&f.text).unwrap();
    assert!((caret.y + caret.height / 2. - (inner.y + inner.height / 2.)).abs() < 2.);
    e.hit(&mut f.text, f.chain, Vec2::new(inner.x + 1., inner.y + inner.height / 2.), false);
    assert_eq!(e.caret.byte_index, 0, "clicks use the same centered origin as drawing");

    let mut ordinary = Editor::new("hello".into());
    f.view(&mut ordinary, 30., 2., false);
    assert_eq!(ordinary.origin(f.layout(&ordinary), ordinary.view.unwrap()).y,
        ordinary.view.unwrap().inner().y, "large multiline editors keep their original top alignment");

    for (value, width) in [("hello\nworld", 30.), ("hello world", 4.)] {
        let mut e = Editor::composer(value.into());
        f.view(&mut e, width, 8., false);
        let layout = f.layout(&e);
        assert!(layout.line_count() > 1);
        let view = e.view.unwrap();
        assert_eq!(e.origin(layout, view).y, view.inner().y,
            "hard lines and wraps stay top-aligned");
    }
    e.replace("\nworld");
    f.view(&mut e, 30., 8., false);
    assert_eq!(e.origin(f.layout(&e), e.view.unwrap()).y, e.view.unwrap().inner().y);
    assert!(f.key(&mut e, "z", true, false));
    f.view(&mut e, 30., 2., false);
    let view = e.view.unwrap();
    assert!(e.origin(f.layout(&e), view).y > view.inner().y, "undo recenters one line");
}

#[test]
fn up_down_keep_the_goal_across_short_lines_and_do_no_shaping_or_text_edits() {
    let mut f = Fixture::new();
    let mut e = Editor::new("abcdefghij\nab\nabcdefghij".into());
    f.view(&mut e, 80., 8., false);
    f.key(&mut e, "Home", true, false);
    for _ in 0..9 { f.key(&mut e, "ArrowRight", false, false); }
    assert_eq!(e.caret.byte_index, 9);
    let original = e.value.clone();
    sanscale::profiling::reset_work_counters();
    assert!(!f.key(&mut e, "ArrowDown", false, false));
    assert_eq!(e.caret.byte_index, 13);
    let goal = e.goal;
    f.key(&mut e, "ArrowDown", false, false);
    assert_eq!(e.caret.byte_index, 23);
    assert_eq!(e.goal, goal);
    f.key(&mut e, "ArrowUp", false, false);
    assert_eq!(e.caret.byte_index, 13);
    f.key(&mut e, "ArrowLeft", false, false);
    assert_eq!(e.goal, None);
    f.key(&mut e, "ArrowDown", false, false);
    assert_eq!(e.caret.byte_index, 15);
    assert_eq!(e.value, original);
    assert!(e.undo.is_empty());
    let work = sanscale::profiling::work_counters();
    assert_eq!((work.block_requests, work.shape_calls, work.flow_calls), (0, 0, 0));
}

#[test]
fn word_motion_deletion_selection_collapse_and_undo_are_one_edit_path() {
    let mut f = Fixture::new();
    let mut e = Editor::new("hello_world, e\u{301}lan 😀\nnext".into());
    f.view(&mut e, 80., 10., false);
    f.key(&mut e, "Home", true, false);
    f.key(&mut e, "ArrowRight", true, false);
    assert_eq!(e.caret.byte_index, "hello_world".len());
    f.key(&mut e, "ArrowRight", true, true);
    assert_eq!(e.selected(), ",");
    f.key(&mut e, "ArrowLeft", false, false);
    assert!(e.range().is_empty());
    assert_eq!(e.caret.byte_index, 11, "collapse without an extra cluster step");
    assert!(f.key(&mut e, "Delete", true, false));
    assert_eq!(e.value, "hello_world e\u{301}lan 😀\nnext");
    assert!(f.key(&mut e, "z", true, false));
    assert_eq!(e.value, "hello_world, e\u{301}lan 😀\nnext");
    assert!(e.range().is_empty(), "undo restores the pre-delete caret, not a fabricated selection");
    f.key(&mut e, "End", true, false);
    assert!(f.key(&mut e, "Backspace", true, false));
    assert!(e.value.ends_with('\n'));
    f.key(&mut e, "z", true, false);
    assert!(e.value.ends_with("next"));
    f.key(&mut e, "y", true, false);
    assert!(e.value.ends_with('\n'));
}

#[test]
fn mouse_and_keys_preserve_both_sides_of_a_soft_wrap_and_cross_hard_lines() {
    let mut f = Fixture::new();
    let mut e = Editor::new("abcd efgh ijkl\nlast".into());
    f.view(&mut e, 3.2, 20., false);
    let layout = f.layout(&e);
    let first = layout.line(0).unwrap();
    let seam = layout.line_range(0).unwrap().end;
    assert_eq!(seam, layout.line_range(1).unwrap().start, "fixture must soft-wrap");
    let view = e.view.unwrap(); let origin = e.origin(layout, view);
    let point = Vec2::new(origin.x + (first.width_em + 0.3) * view.size, origin.y + first.height_em * view.size * 0.5);
    e.hit(&mut f.text, f.chain, point, false);
    assert_eq!((e.caret.byte_index, e.caret.line_index), (seam, 0));
    f.key(&mut e, "ArrowRight", false, false);
    assert_eq!(e.caret.line_index, 1);
    f.key(&mut e, "ArrowLeft", false, false);
    assert_eq!((e.caret.byte_index, e.caret.line_index), (seam, 1));
    let caret = f.layout(&e).caret_rect(e.placed(f.layout(&e), false));
    assert_eq!(caret.y_em, f.layout(&e).line(1).unwrap().top_em);
    f.key(&mut e, "ArrowLeft", false, false);
    assert_eq!(e.caret.line_index, 0);
    f.key(&mut e, "End", true, false);
    f.key(&mut e, "Home", false, false);
    let start = e.value.find("last").unwrap();
    assert_eq!(e.caret.byte_index, start);
    f.key(&mut e, "ArrowLeft", false, false);
    assert_eq!(e.caret.byte_index, start - 1);
    f.key(&mut e, "ArrowRight", false, false);
    assert_eq!(e.caret.byte_index, start);
}

#[test]
fn reflow_reanchors_a_hard_break_and_post_edit_placement_is_end_affine() {
    let mut f = Fixture::new(); let mut e = Editor::new("ab\ncd".into());
    f.view(&mut e, 0.7, 12., false);
    f.key(&mut e, "Home", true, false);
    f.key(&mut e, "ArrowRight", false, false); f.key(&mut e, "ArrowRight", false, false);
    assert_eq!((e.caret.byte_index, e.caret.line_index), (2, 1));
    f.view(&mut e, 20., 12., false);
    assert_eq!((e.caret.byte_index, e.caret.line_index), (2, 0));
    e = Editor::new("ab".into()); f.view(&mut e, 0.7, 12., false);
    f.key(&mut e, "Home", true, false);
    assert!(e.replace("x")); f.view(&mut e, 0.7, 12., false);
    assert_eq!(e.caret, f.layout(&e).caret_after_edit(1));
}

#[test]
fn unicode_masking_and_cluster_delete_never_split_text_or_undo_selections() {
    let mut f = Fixture::new();
    for secret in [false, true] {
        let value = "e\u{301}😀👩‍💻";
        let mut e = Editor::line(value.into());
        f.view(&mut e, 20., 4., secret);
        f.key(&mut e, "Home", true, false);
        for end in ["e\u{301}".len(), "e\u{301}😀".len(), value.len()] {
            f.key(&mut e, "ArrowRight", false, false);
            assert_eq!(e.caret.byte_index, end);
        }
        assert!(f.key(&mut e, "Backspace", false, false));
        assert_eq!(e.value, "e\u{301}😀");
        f.key(&mut e, "z", true, false);
        assert_eq!(e.value, value);
        assert_eq!(e.caret.byte_index, value.len());
        assert!(e.range().is_empty());
        if secret {
            assert!(f.key(&mut e, "Backspace", true, false));
            assert!(e.value.is_empty(), "masked input is one word");
        }
    }
    let mut e = Editor::line("token".into());
    assert!(e.replace("\r\nmore\rtext"));
    assert_eq!(e.value, "tokenmoretext");
}

#[test]
fn preedit_is_a_projection_and_one_commit_replaces_the_original_selection() {
    let mut f = Fixture::new(); let mut e = Editor::new("hello world".into());
    f.view(&mut e, 40., 6., false);
    f.key(&mut e, "ArrowLeft", true, true);
    assert_eq!(e.selected(), "world");
    e.preedit("世".into(), Some((3, 3)));
    e.preedit("世界".into(), Some((3, 6)));
    f.view(&mut e, 40., 6., false);
    assert_eq!(e.display().0, "hello 世界");
    assert_eq!(e.display().1, "hello 世界".len());
    assert_eq!(e.value, "hello world");
    assert!(e.undo.is_empty());
    assert!(!f.key(&mut e, "Enter", false, false));
    assert!(!f.key(&mut e, "Backspace", false, false));
    assert!(e.replace("世界"));
    assert_eq!(e.value, "hello 世界");
    assert!(!e.composing());
    assert_eq!(e.undo.len(), 1);
    f.key(&mut e, "z", true, false);
    assert_eq!(e.value, "hello world");
    assert_eq!(e.selected(), "world");
    e.preedit("cancel".into(), None);
    e.preedit(String::new(), None);
    assert_eq!(e.value, "hello world");
}

#[test]
fn wheel_stays_independent_until_navigation_and_hit_testing_uses_the_scrolled_origin() {
    let mut f = Fixture::new(); let mut e = Editor::new((0..40).map(|i| format!("line {i:02}\n")).collect());
    f.view(&mut e, 30., 4., false);
    assert!(e.scroll.y > 0.);
    e.wheel(&mut f.text, f.chain, -100_000., false);
    assert_eq!(e.scroll.y, 0.);
    for _ in 0..3 { e.prepare_view(&mut f.text, f.chain, true); assert_eq!(e.scroll.y, 0., "idle must not snap back to caret"); }
    e.wheel(&mut f.text, f.chain, 90., false);
    let view = e.view.unwrap(); let point = Vec2::new(view.inner().x + 4., view.inner().y + 8.);
    let origin = e.origin(f.layout(&e), view);
    let expected = f.layout(&e).hit_test(Vec2::new((point.x - origin.x) / view.size, (point.y - origin.y) / view.size)).unwrap();
    e.hit(&mut f.text, f.chain, point, false);
    assert_eq!(e.caret, expected);
    let anchor = e.anchor;
    for _ in 0..20 { e.drag_scroll(&mut f.text, f.chain, Vec2::new(point.x + 10., view.inner().y + view.inner().height + 40.), 1. / 60.); }
    assert_eq!(e.anchor, anchor);
    assert!(e.caret.byte_index > anchor);
    f.key(&mut e, "Home", true, false);
    e.prepare_view(&mut f.text, f.chain, true);
    assert_eq!(e.scroll.y, 0.);
    f.key(&mut e, "PageDown", false, true);
    assert!(e.caret.line_index > 0);
    assert_eq!(e.anchor, 0);
}

#[test]
fn a_cleared_service_recovers_the_editor_layout_without_losing_text() {
    let mut f = Fixture::new(); let mut e = Editor::new("kept document\nnext line".into());
    f.view(&mut e, 20., 10., false);
    let old = e.layout.as_ref().unwrap().block;
    f.text.clear();
    let font = f.text.map_font(Arc::new(include_bytes!("../../assets/DejaVuSansMono.ttf").as_slice()), 0).unwrap();
    f.chain = f.text.register_chain(&[font]).unwrap();
    f.view(&mut e, 20., 10., false);
    assert_eq!(f.text.measure(old).line_count(), 0);
    assert_eq!(f.layout(&e).len_bytes(), e.value.len());
    assert_eq!(f.layout(&e).line_count(), 2);
}

#[test]
fn ligatures_do_not_turn_one_backspace_into_deleting_several_letters() {
    let mut f = Fixture::new();
    let font = f.text.map_font(Arc::new(include_bytes!("../../assets/DejaVuSans.ttf").as_slice()), 0).unwrap();
    f.chain = f.text.register_chain(&[font]).unwrap();
    let mut e = Editor::new("office".into());
    f.view(&mut e, 30., 4., false);
    let mut stops = vec![0];
    while let Some(next) = f.layout(&e).next_caret_stop(*stops.last().unwrap()) { stops.push(next); }
    assert_eq!(stops.first(), Some(&0));
    assert_eq!(stops.last(), Some(&e.value.len()));
    for expected in ["offic", "offi", "off", "of", "o", ""] {
        assert!(f.key(&mut e, "Backspace", false, false));
        assert_eq!(e.value, expected);
    }
}


#[test]
fn inserting_a_base_before_a_combining_mark_places_after_the_new_grapheme() {
    let mut f = Fixture::new(); let mut e = Editor::new("\u{301} end".into());
    f.view(&mut e, 30., 6., false);
    f.key(&mut e, "Home", true, false);
    e.replace("e");
    assert_eq!(e.value, "e\u{301} end");
    assert_eq!(e.caret.byte_index, "e\u{301}".len());
    f.key(&mut e, "Backspace", false, false);
    assert_eq!(e.value, " end");
}
