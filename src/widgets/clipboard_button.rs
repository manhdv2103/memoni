use egui::{
    CornerRadius, Image, Rect, Response, Sense, Stroke, StrokeKind, TextStyle, TextWrapMode,
    TextureHandle, Ui, Vec2, Widget, WidgetText,
};

pub struct ClipboardButton {
    label: WidgetText,
    image: Option<(TextureHandle, Vec2)>,
    is_active: bool,
}

impl ClipboardButton {
    pub fn new(label: impl Into<WidgetText>) -> Self {
        Self {
            label: label.into(),
            image: None,
            is_active: false,
        }
    }

    #[inline]
    pub fn label(mut self, label: impl Into<WidgetText>) -> Self {
        self.label = label.into();
        self
    }

    #[inline]
    pub fn image(mut self, texture: TextureHandle, size: Vec2) -> Self {
        self.image = Some((texture, size));
        self
    }

    #[inline]
    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }
}

impl Widget for ClipboardButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let padding = ui.style().spacing.button_padding;

        let desired_width = ui.available_width();

        let mut text_width = desired_width - padding.x * 2.0;
        if let Some((_, img_size)) = self.image {
            text_width -= img_size.x;
        }
        let galley = self.label.into_galley(
            ui,
            Some(TextWrapMode::Truncate),
            text_width,
            TextStyle::Button,
        );

        let mut desired_height = 0.0;
        let text_height = galley.size().y;
        let image_height = self.image.as_ref().map(|i| i.1.y).unwrap_or(0.0);
        desired_height += image_height.max(text_height + padding.y * 2.0);

        let (rect, response) =
            ui.allocate_at_least(Vec2::new(desired_width, desired_height), Sense::CLICK);

        if ui.is_rect_visible(rect) {
            let visuals = &ui.style().visuals.widgets.inactive;
            let bg_fill = if self.is_active {
                ui.style().visuals.widgets.active.weak_bg_fill
            } else {
                visuals.weak_bg_fill
            };

            ui.painter().rect(
                rect,
                visuals.corner_radius,
                bg_fill,
                Stroke::NONE,
                StrokeKind::Inside,
            );

            let mut cursor_x = rect.min.x;
            if let Some((texture, size)) = self.image {
                let image_rect = Rect::from_min_size(rect.min, size);
                let image = Image::from_texture(&texture)
                    .maintain_aspect_ratio(true)
                    .corner_radius(CornerRadius {
                        nw: visuals.corner_radius.nw,
                        sw: visuals.corner_radius.sw,
                        ..Default::default()
                    });

                image.paint_at(ui, image_rect);

                cursor_x += image_rect.width();
            }

            let mut text_pos = ui
                .layout()
                .align_size_within_rect(galley.size(), rect.shrink2(padding))
                .min;
            text_pos.x = cursor_x + padding.x;
            ui.painter().galley(text_pos, galley, visuals.text_color());
        }

        response
    }
}
