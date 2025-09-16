use std::{ffi::CString, fs, path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use egui::{FontData, FontDefinitions, FontFamily, FontTweak, FullOutput, RawInput};
use fontconfig::Fontconfig;

use crate::{config::Config, selection::SelectionItem, utils::is_plaintext_mime};

pub struct Ui<'a> {
    pub egui_ctx: egui::Context,
    config: &'a Config,
}

impl<'a> Ui<'a> {
    pub fn new(config: &'a Config) -> Result<Self> {
        let egui_ctx = egui::Context::default();
        let styling = &config.style;

        egui_ctx.style_mut(|style| {
            // style.debug.debug_on_hover = true;
            style.spacing.button_padding =
                egui::vec2(styling.button_padding_x, styling.button_padding_y);
            style.spacing.item_spacing = egui::vec2(styling.item_spacing_x, styling.item_spacing_y);
            style.interaction.selectable_labels = false;
            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
                *font_id = egui::FontId::proportional(styling.font_size);
            }
        });

        if let Some(font_family) = &styling.font_family {
            if let Some(font_path) = Self::find_font(font_family)? {
                let mut fonts = FontDefinitions::default();
                fonts.font_data.insert(
                    "config_font".to_owned(),
                    Arc::new(FontData::from_owned(fs::read(font_path)?).tweak(FontTweak {
                        baseline_offset_factor: styling.font_baseline_offset_factor,
                        ..Default::default()
                    })),
                );

                fonts
                    .families
                    .get_mut(&FontFamily::Proportional)
                    .unwrap()
                    .insert(0, "config_font".to_owned());

                egui_ctx.set_fonts(fonts);
            } else {
                eprintln!("Warning: font family '{}' not found", font_family);
            }
        }

        Ok(Ui { egui_ctx, config })
    }

    fn find_font(font_family: &str) -> Result<Option<PathBuf>> {
        let fc = Fontconfig::new().ok_or(anyhow!("failed to initialize fontconfig"))?;

        let mut pat = fontconfig::Pattern::new(&fc);
        let family = CString::new(font_family)?;
        pat.add_string(fontconfig::FC_FAMILY, &family);
        pat.add_integer(fontconfig::FC_WEIGHT, fontconfig::FC_WEIGHT_REGULAR);
        pat.add_integer(fontconfig::FC_SLANT, fontconfig::FC_SLANT_ROMAN);
        pat.add_integer(fontconfig::FC_WIDTH, fontconfig::FC_WIDTH_NORMAL);

        let fonts = fontconfig::list_fonts(&pat, None);
        let font = fonts.iter().next();

        Ok(font.and_then(|f| f.filename().map(PathBuf::from)))
    }

    pub fn run<'b, I: IntoIterator<Item = &'b SelectionItem>>(
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
        let corner_radius = self.config.style.button_corner_radius;
        let padding = self.config.style.window_padding;
        let scroll_bar_margin = self.config.style.scroll_bar_margin;

        let full_output = self.egui_ctx.run(egui_input, |ctx| {
            run_result = Self::container(ctx, padding, scroll_bar_margin, |ui| {
                for item in &render_items {
                    if ui
                        .add(
                            egui::Button::new(item.0)
                                .truncate()
                                .corner_radius(egui::CornerRadius::same(corner_radius))
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
        padding: i8,
        scroll_bar_margin: f32,
        add_contents: impl FnOnce(&mut egui::Ui) -> Result<()>,
    ) -> Result<()> {
        let mut err: Option<anyhow::Error> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ctx, |ui| {
                let scroll_bar_rect = egui::Rect::from_min_max(
                    ui.min_rect().min + egui::vec2(0.0, scroll_bar_margin),
                    ui.max_rect().max - egui::vec2(0.0, scroll_bar_margin),
                );
                egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .scroll_bar_rect(scroll_bar_rect)
                    .show(ui, |ui| {
                        egui::Frame::new()
                            .inner_margin(egui::Margin::symmetric(padding, padding))
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
