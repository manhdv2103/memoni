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
        mut on_paste: impl FnMut(&SelectionItem),
    ) -> Result<FullOutput> {
        let mut render_items = vec![];
        for item in selection_items {
            if let Some((_, value)) = item.data.iter().find(|(k, _)| is_plaintext_mime(k)) {
                render_items.push((str::from_utf8(value)?, item));
            }
        }

        let mut run_result: Result<()> = Ok(());
        let full_output = self.egui_ctx.run(egui_input, |ctx| {
            run_result = Self::container(ctx, |ui| {
                for item in &render_items {
                    if ui
                        .add(
                            egui::Button::new(item.0)
                                .truncate()
                                .corner_radius(egui::CornerRadius::same(7))
                                .min_size(egui::vec2(ui.available_width(), 0.0)),
                        )
                        .clicked()
                    {
                        on_paste(item.1);
                    }
                }

                Ok(())
            });
        });

        match run_result {
            Ok(_) => Ok(full_output),
            Err(err) => Err(err),
        }
    }

    fn container(
        ctx: &egui::Context,
        add_contents: impl FnOnce(&mut egui::Ui) -> Result<()>,
    ) -> Result<()> {
        let mut err: Option<anyhow::Error> = None;

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
                            .show(ui, |ui| {
                                if let Err(e) = add_contents(ui) {
                                    err = Some(e);
                                }
                            });
                    });
            });

        match err {
            None => Ok(()),
            Some(e) => Err(e),
        }
    }
}
