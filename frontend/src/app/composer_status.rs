use super::*;

impl App {
    /// Reserve the thinking label before shortening a long provider/model slug.
    /// Both desktop and phone use this row; missing cached metadata is not a default.
    pub(super) fn composer_model_status(&mut self, layer: &mut Layer, summary: Option<&SessionSummary>, rect: Rect) {
        let model = summary.and_then(|s| s.model.as_ref())
            .map(|m| format!("{}/{}", m.provider, m.model_id))
            .unwrap_or_else(|| "Model: unknown".into());
        let level = summary.and_then(|s| s.thinking_level.as_deref())
            .filter(|level| !level.is_empty()).unwrap_or("unknown");
        let thinking = format!("Thinking: {level}");
        let size = 12. * self.ui.scale;
        let gap = 12. * self.ui.scale;
        let thinking_width = self.services.renderer.label_width(&thinking, size, false).ceil();
        let model_width = self.services.renderer.label_width(&model, size, false).ceil()
            .min((rect.width - thinking_width - gap).max(0.));
        self.services.renderer.ellipsized_label(layer, &model,
            Rect::new(rect.x, rect.y, model_width, rect.height),
            size, color(0x82909f), false, false, rect);
        self.services.renderer.label(layer, &thinking,
            Rect::new(rect.x + model_width + gap, rect.y, thinking_width, rect.height),
            size, color(0xb7c2ce), false);
    }
}
