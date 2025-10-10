use egui::{
    Color32, CornerRadius, Image, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextStyle,
    TextWrapMode, TextureHandle, Ui, Vec2, Widget, WidgetText,
};

#[derive(Default)]
pub struct ClipboardButton {
    labels: Vec<WidgetText>,
    sublabel: Option<WidgetText>,
    image: Option<(TextureHandle, Vec2)>,
    image_source: Option<String>,
    image_background: Color32,
    is_active: bool,
    with_preview_padding: Option<Vec2>,
    underline_offset: f32,
}

impl ClipboardButton {
    #[inline]
    pub fn label(mut self, label: impl Into<WidgetText>) -> Self {
        self.labels = vec![label.into()];
        self
    }

    #[inline]
    pub fn append_label(mut self, label: impl Into<WidgetText>) -> Self {
        self.labels.push(label.into());
        self
    }

    #[inline]
    pub fn sublabel(mut self, sublabel: impl Into<WidgetText>) -> Self {
        self.sublabel = Some(sublabel.into());
        self
    }

    #[inline]
    pub fn image(mut self, texture: TextureHandle, size: impl Into<Vec2>) -> Self {
        self.image = Some((texture, size.into()));
        self
    }

    #[inline]
    pub fn image_source(mut self, image_source: &str) -> Self {
        self.image_source = Some(image_source.to_string());
        self
    }

    #[inline]
    pub fn image_background(mut self, image_background: impl Into<Color32>) -> Self {
        self.image_background = image_background.into();
        self
    }

    #[inline]
    pub fn is_active(mut self, is_active: bool) -> Self {
        self.is_active = is_active;
        self
    }

    #[inline]
    pub fn with_preview_padding(mut self, with_preview_padding: impl Into<Vec2>) -> Self {
        self.with_preview_padding = Some(with_preview_padding.into());
        self
    }

    #[inline]
    pub fn underline_offset(mut self, underline_offset: f32) -> Self {
        self.underline_offset = underline_offset;
        self
    }
}

impl Widget for ClipboardButton {
    fn ui(self, ui: &mut Ui) -> Response {
        let padding = if self.image.is_some()
            && let Some(with_preview_padding) = self.with_preview_padding
        {
            with_preview_padding
        } else {
            ui.style().spacing.button_padding
        };

        let desired_width = ui.available_width();

        let mut text_width = desired_width - padding.x * 2.0;
        if let Some((_, img_size)) = self.image {
            text_width -= img_size.x;
        }
        let galleys = self.labels.into_iter().map(|l| {
            l.into_galley(
                ui,
                Some(TextWrapMode::Truncate),
                text_width,
                TextStyle::Button,
            )
        }).collect::<Vec<_>>();
        let sublabel_galley = self.sublabel.map(|sl| {
            sl.into_galley(
                ui,
                Some(TextWrapMode::Truncate),
                text_width,
                TextStyle::Button,
            )
        });
        let img_src_galley = if self.image.is_some() {
            self.image_source.as_ref().map(|s| {
                Into::<WidgetText>::into(s).into_galley(
                    ui,
                    Some(TextWrapMode::Truncate),
                    text_width,
                    TextStyle::Button,
                )
            })
        } else {
            None
        };

        let mut desired_height = 0.0;
        let text_height = galleys.iter().fold(0.0, |acc, g| acc + g.size().y)
            + sublabel_galley.as_ref().map(|g| g.size().y).unwrap_or(0.0)
            + img_src_galley.as_ref().map(|g| g.size().y).unwrap_or(0.0);
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
            if let Some((ref texture, size)) = self.image {
                let image_rect = Rect::from_min_size(rect.min, size);
                let image = Image::from_texture(texture)
                    .maintain_aspect_ratio(true)
                    .bg_fill(self.image_background)
                    .corner_radius(CornerRadius {
                        nw: visuals.corner_radius.nw,
                        sw: visuals.corner_radius.sw,
                        ..Default::default()
                    });

                image.paint_at(ui, image_rect);

                cursor_x += image_rect.width();
            }

            cursor_x += padding.x;
            let mut cursor_y = rect.min.y + padding.y;
            for galley in galleys {
                let text_pos = Pos2::new(cursor_x, cursor_y);
                cursor_y += galley.size().y;
                ui.painter().galley(text_pos, galley, visuals.text_color());
            }

            if let Some(galley) = img_src_galley {
                let text_pos = Pos2::new(cursor_x, cursor_y);
                let text_underline = Stroke {
                    width: 1.0,
                    color: visuals.text_color(),
                };

                // Drawing text underline manually with offset to workaround https://github.com/emilk/egui/issues/5855
                let underline_y =
                    text_pos.y + galley.size().y - text_underline.width + self.underline_offset;
                ui.painter().line_segment(
                    [
                        Pos2::new(text_pos.x, underline_y),
                        Pos2::new(text_pos.x + galley.size().x, underline_y),
                    ],
                    text_underline,
                );
                ui.painter().galley(text_pos, galley, visuals.text_color());
            }

            if let Some(galley) = sublabel_galley {
                let text_pos =
                    Pos2::new(cursor_x, rect.shrink2(padding).bottom() - galley.size().y);
                ui.painter().galley(text_pos, galley, visuals.text_color());
            }
        }

        response
    }
}
