use super::controls::{ButtonStyle, Form};
use super::{Context, Controller, Event, Frame, Id, Request, Target, UiState, Widget};
use crate::{
    notice::DownloadTarget,
    render::{color, contains},
};
use sanscale::{Rect, Vec2};
use std::path::PathBuf;

pub(in crate::app) struct ImageSpec {
    pub path: PathBuf,
    pub name: String,
    pub target: DownloadTarget,
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Choice {
    Back,
    Smaller,
    Fit,
    Larger,
    Save,
}
pub(in crate::app) struct ImageViewer {
    pub id: Id,
    spec: ImageSpec,
    form: Form<Choice>,
    pub zoom: f32,
    pub pan: Vec2,
    pub image: Option<Rect>,
    pointer: Option<(u64, Vec2, Vec2, bool)>,
    second: Option<(u64, Vec2)>,
}
impl ImageViewer {
    pub fn new(spec: ImageSpec) -> Self {
        let id = Id::new();
        Self {
            id,
            spec,
            form: Form::new(
                id,
                &[
                    (Choice::Back, "Back"),
                    (Choice::Smaller, "−"),
                    (Choice::Fit, "Fit"),
                    (Choice::Larger, "+"),
                    (Choice::Save, "↓"),
                ],
            ),
            zoom: 1.,
            pan: Vec2::new(0., 0.),
            image: None,
            pointer: None,
            second: None,
        }
    }
    #[cfg(test)]
    pub fn button(&self, name: &str) -> Rect {
        self.form.buttons.iter().find(|(_, b)| b.label == name).unwrap().1.control.rect.unwrap()
    }
}
impl Widget for ImageViewer {
    fn owns(&self, target: Target, model: &Controller, _ui: &UiState) -> bool {
        self.spec.target.matches_source(&model.identity, model.account.source_lineage.as_deref())
            && self.form.owns(target)
    }

    fn handle_event(&mut self, event: &Event<'_>, cx: &mut Context<'_>) -> bool {
        if !self.spec.target.matches_source(&cx.model.identity, cx.model.account.source_lineage.as_deref()) {
            cx.ui.requests.push_back(Request::CloseViewer(self.id));
            return true;
        }
        let (handled, choice) = match event {
            Event::Back | Event::Key { key: "Escape", .. } => (true, Some(Choice::Back)),
            _ if self.pointer.is_none() => self.form.event(event, std::iter::empty(), cx),
            _ => (false, None),
        };
        if let Some(choice) = choice {
            match choice {
                Choice::Back => cx.ui.requests.push_back(Request::CloseViewer(self.id)),
                Choice::Smaller => self.zoom = (self.zoom * 0.8).max(1.),
                Choice::Larger => self.zoom = (self.zoom * 1.25).min(16.),
                Choice::Fit => {
                    self.zoom = 1.;
                    self.pan = Vec2::new(0., 0.);
                }
                Choice::Save => cx.begin_save(
                    &self.spec.target.session,
                    &self.spec.target.entry,
                    self.spec.path.clone(),
                    self.spec.name.clone(),
                ),
            }
            cx.ui.dirty = true;
            return true;
        }
        if handled {
            return true;
        }
        match *event {
            Event::Cancel => {
                self.pointer = None;
                self.second = None;
            }
            Event::Down { pointer, point, .. } if cx.ui.capture.is_none() => {
                if let Some((id, _, _, _)) = self.pointer {
                    if id != pointer && self.second.is_none() {
                        self.second = Some((pointer, point));
                    }
                } else {
                    self.pointer = Some((pointer, point, point, false));
                }
            }
            Event::Move { pointer, point } => {
                if let Some((id, start, last, dragged)) = &mut self.pointer {
                    if let Some((second, other)) = &mut self.second {
                        let distance = |a: Vec2, b: Vec2| ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt();
                        let old = distance(*last, *other);
                        if pointer == *id {
                            *last = point;
                        } else if pointer == *second {
                            *other = point;
                        } else {
                            return true;
                        }
                        if old > 1. {
                            self.zoom = (self.zoom * distance(*last, *other) / old).clamp(1., 16.);
                        }
                        *dragged = true;
                    } else if pointer == *id {
                        let dx = point.x - last.x;
                        let dy = point.y - last.y;
                        *dragged |= (point.x - start.x).hypot(point.y - start.y) > 7. * cx.ui.scale;
                        self.pan.x += dx;
                        self.pan.y += dy;
                        *last = point;
                    }
                    cx.ui.dirty = true;
                }
            }
            Event::Up { pointer, point }
                if self.pointer.is_some_and(|(id, _, _, _)| id == pointer)
                    || self.second.is_some_and(|(id, _)| id == pointer) =>
            {
                if let Some((_, start, _, dragged)) = self.pointer.take() {
                    if self.second.take().is_none()
                        && !dragged
                        && self.image.is_none_or(|r| !contains(r, start) && !contains(r, point))
                    {
                        cx.ui.requests.push_back(Request::CloseViewer(self.id));
                    }
                }
            }
            Event::Wheel { amount, .. } => {
                self.zoom = (self.zoom * (-amount * 0.002).exp()).clamp(1., 16.);
                cx.ui.dirty = true;
            }
            _ => {}
        }
        true
    }
    fn visit_perframe(&mut self, frame: &mut Frame<'_>, cx: &mut Context<'_>) {
        let b = frame.bounds;
        let s = cx.ui.scale;
        self.form.begin_frame();
        self.image = None;
        frame.layer.above();
        frame.layer.rect(b, color(0x06090d));
        match cx.services.renderer.image_size(&cx.services.gpu, &self.spec.path) {
            Ok((w, h)) => {
                let fit = (b.width / w as f32).min((b.height - 100. * s) / h as f32);
                let width = w as f32 * fit * self.zoom;
                let height = h as f32 * fit * self.zoom;
                let rect = Rect::new(
                    b.x + (b.width - width) / 2. + self.pan.x,
                    b.y + 60. * s + (b.height - 100. * s - height) / 2. + self.pan.y,
                    width,
                    height,
                );
                let clip = Rect::new(b.x, b.y + 56. * s, b.width, (b.height - 100. * s).max(0.));
                self.image = Some(crate::render::intersect(rect, clip));
                frame.layer.images.push((self.spec.path.clone(), rect, clip));
            }
            Err(error) => {
                cx.services.renderer.label(
                    frame.layer,
                    &error,
                    Rect::new(b.x + 20. * s, b.y + 80. * s, b.width - 40. * s, 100. * s),
                    15. * s,
                    color(0xffb4ab),
                    false,
                );
            }
        }
        frame.layer.above();
        for (i, choice) in
            [Choice::Back, Choice::Smaller, Choice::Fit, Choice::Larger, Choice::Save].into_iter().enumerate()
        {
            self.form.button(
                choice,
                Rect::new(b.x + (12. + i as f32 * 66.) * s, b.y + 8. * s, 60. * s, 38. * s),
                ButtonStyle::Tonal,
                frame,
                cx,
            );
        }
    }
}
