use anyhow::Result;
use egui::{FullOutput, RawInput};

use crate::{selection::SelectionItem, utils::is_plaintext_mime};

pub struct Ui {
    pub egui_ctx: egui::Context,
}

impl Ui {
    pub fn new() -> Result<Self> {
        let egui_ctx = egui::Context::default();

        egui_ctx.style_mut(|style| {
            // style.debug.debug_on_hover = true;
            style.spacing.button_padding = egui::vec2(8.0, 8.0);
            style.spacing.item_spacing = egui::vec2(5.0, 5.0);
            style.interaction.selectable_labels = false;
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
                *font_id = egui::FontId::proportional(13.0);
            }
        });

        Ok(Ui { egui_ctx })
    }

    pub fn run<'a, I: IntoIterator<Item = &'a SelectionItem>>(
        &self,
        egui_input: RawInput,
        selection_items: I,
    ) -> Result<FullOutput> {
        let mut render_items = vec![];
        for item in selection_items {
            if let Some((_, value)) = item.data.iter().find(|(k, _)| is_plaintext_mime(k)) {
                render_items.push(str::from_utf8(value)?);
            }
        }

        let full_output = self.egui_ctx.run(egui_input, |ctx| {
            Self::container(ctx, |ui| {
                for item in &render_items {
                    let _ = ui.add(
                        egui::Button::new(*item)
                            .truncate()
                            .corner_radius(egui::CornerRadius::same(7))
                            .min_size(egui::vec2(ui.available_width(), 0.0)),
                    );
                }
            });
        });

        Ok(full_output)
    }

    fn container(ctx: &egui::Context, add_contents: impl FnOnce(&mut egui::Ui)) {
        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ctx, |ui| {
                let scroll_bar_rect = egui::Rect::from_min_max(
                    ui.min_rect().min + egui::vec2(0.0, 8.0),
                    ui.max_rect().max - egui::vec2(0.0, 8.0),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .scroll_bar_rect(scroll_bar_rect)
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(8, 8))
                            .show(ui, add_contents);
                    });
            });
    }
}
