use std::{ffi::CString, fs, path::PathBuf, sync::Arc};

use anyhow::{Result, anyhow};
use egui::{
    FontData, FontDefinitions, FontFamily, FontTweak, FullOutput, RawInput, Stroke,
    scroll_area::ScrollAreaOutput,
};
use fontconfig::Fontconfig;

use crate::{
    config::{Config, LayoutConfig, ThemeConfig},
    selection::SelectionItem,
    utils::is_plaintext_mime,
};

pub struct Ui<'a> {
    pub egui_ctx: egui::Context,
    config: &'a Config,
    item_ids: Vec<egui::Id>,
    hovered_idx: Option<usize>,
    active_idx: usize,
    scroll_area_output: Option<ScrollAreaOutput<()>>,
}

impl<'a> Ui<'a> {
    pub fn new(config: &'a Config) -> Result<Self> {
        let egui_ctx = egui::Context::default();
        let layout = &config.layout;
        let font = &config.font;
        let theme = &config.theme;

        egui_ctx.style_mut(|style| {
            // style.debug.debug_on_hover = true;
            style.spacing.button_padding =
                egui::vec2(layout.button_padding.x, layout.button_padding.y);
            style.spacing.item_spacing = egui::vec2(layout.item_spacing.x, layout.item_spacing.y);
            style.interaction.selectable_labels = false;

            style.visuals.override_text_color = Some(theme.foreground.into());
            for widget in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
            ] {
                widget.fg_stroke.color = theme.foreground.into();
                widget.weak_bg_fill = theme.button_background.into();
                widget.bg_stroke = Stroke::NONE;
                widget.expansion = 0.0;
            }

            if let Some(font_id) = style.text_styles.get_mut(&egui::TextStyle::Body) {
                *font_id = egui::FontId::proportional(font.size);
            }
        });

        if let Some(font_family) = &font.family {
            if let Some(font_path) = Self::find_font(font_family)? {
                let mut fonts = FontDefinitions::default();
                fonts.font_data.insert(
                    "config_font".to_owned(),
                    Arc::new(FontData::from_owned(fs::read(font_path)?).tweak(FontTweak {
                        baseline_offset_factor: font.baseline_offset_factor,
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

        Ok(Ui {
            egui_ctx,
            config,
            item_ids: vec![],
            hovered_idx: None,
            active_idx: 0,
            scroll_area_output: None,
        })
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
        &mut self,
        mut egui_input: RawInput,
        selection_items: I,
        mut on_paste: impl FnMut(&SelectionItem),
    ) -> Result<FullOutput> {
        let mut render_items = vec![];
        for item in selection_items {
            if let Some((_, value)) = item.data.iter().find(|(k, _)| is_plaintext_mime(k)) {
                render_items.push((str::from_utf8(value)?, item));
            }
        }

        let mut run_error = None;
        let corner_radius = self.config.layout.button_corner_radius;
        let layout = &self.config.layout;
        let theme = &self.config.theme;

        let prev_active_idx = self.active_idx;
        let mut move_by_key = false;
        let mut pointer_moved = false;
        egui_input.events.retain(|ev| {
            // We will handle key events ourself
            if let egui::Event::Key {
                key,
                pressed,
                modifiers,
                ..
            } = ev
            {
                if *pressed {
                    let focus_direction = match key {
                        egui::Key::ArrowUp | egui::Key::K => -1,
                        egui::Key::P if modifiers.ctrl => -1,
                        egui::Key::Tab if modifiers.shift => -1,
                        egui::Key::ArrowDown | egui::Key::J => 1,
                        egui::Key::N if modifiers.ctrl => 1,
                        egui::Key::Tab => 1,
                        _ => 0,
                    };
                    if focus_direction != 0 {
                        self.active_idx = (self.active_idx as isize + focus_direction)
                            .rem_euclid(self.item_ids.len() as isize)
                            as usize;
                        move_by_key = true;
                    }

                    if matches!(key, egui::Key::Enter | egui::Key::Space)
                        && let Some(item) = render_items.get(self.active_idx)
                    {
                        on_paste(item.1);
                    }
                }
                return false;
            }

            if matches!(ev, egui::Event::PointerMoved(_)) {
                pointer_moved = true;
            }

            true
        });

        let full_output = self.egui_ctx.run(egui_input, |ctx| {
            if let Some(current_hovered_idx) = ctx.viewport(|vp| {
                self.item_ids
                    .iter()
                    .position(|id| vp.interact_widgets.hovered.contains(id))
            }) {
                if move_by_key {
                    self.hovered_idx = None;
                } else if self.hovered_idx.is_some() || pointer_moved {
                    self.hovered_idx = Some(current_hovered_idx);
                    self.active_idx = current_hovered_idx;
                }
            }

            let mut next_scroll_offset = None;
            if prev_active_idx != self.active_idx
                && let Some(scroll_area) = &self.scroll_area_output
                && let Some(&active_id) = self.item_ids.get(self.active_idx)
                && let Some(active_rect) =
                    ctx.viewport(|vp| vp.prev_pass.widgets.get(active_id).map(|w| w.rect))
            {
                let scroll_rect = scroll_area.inner_rect;
                let scroll_offset = scroll_area.state.offset[1];
                if !scroll_rect.contains_rect(active_rect) {
                    let padding = layout.window_padding.y as f32;
                    next_scroll_offset = Some(if active_rect.top() < scroll_rect.top() + padding {
                        scroll_offset + active_rect.top() - padding
                    } else {
                        scroll_offset + active_rect.bottom() - scroll_rect.height() + padding
                    });
                }
            }

            self.item_ids.clear();
            let container_result = Self::container(ctx, layout, theme, next_scroll_offset, |ui| {
                if render_items.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.add(egui::Label::new("Your clipboard history will appear here."))
                    });
                    return Ok(());
                }

                for (i, item) in render_items.iter().enumerate() {
                    let is_active = i == self.active_idx;

                    // Focused highlight is too flickery, so we will highlight the button ourself
                    let btn_fill = if is_active {
                        theme.button_active_background
                    } else {
                        theme.button_background
                    };

                    let btn = ui.add(
                        egui::Button::new(item.0)
                            .truncate()
                            .corner_radius(egui::CornerRadius::same(corner_radius))
                            .min_size(egui::vec2(ui.available_width(), 0.0))
                            .fill(btn_fill),
                    );
                    self.item_ids.push(btn.id);

                    if btn.clicked() {
                        on_paste(item.1);
                    }
                }

                Ok(())
            });

            match container_result {
                Ok(scroll_area_output) => self.scroll_area_output = Some(scroll_area_output),
                Err(err) => run_error = Some(err),
            }
        });

        match run_error {
            None => Ok(full_output),
            Some(err) => Err(err),
        }
    }

    pub fn reset(&mut self) {
        self.active_idx = 0;
        self.hovered_idx = None;
    }

    fn container(
        ctx: &egui::Context,
        layout: &LayoutConfig,
        theme: &ThemeConfig,
        scroll_offset: Option<f32>,
        add_contents: impl FnOnce(&mut egui::Ui) -> Result<()>,
    ) -> Result<ScrollAreaOutput<()>> {
        let LayoutConfig {
            window_padding: padding,
            scroll_bar_margin,
            ..
        } = layout;
        let mut scroll_area_output = None;
        let mut err: Option<anyhow::Error> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ctx, |ui| {
                let scroll_bar_rect = egui::Rect::from_min_max(
                    ui.min_rect().min + egui::vec2(0.0, *scroll_bar_margin),
                    ui.max_rect().max - egui::vec2(0.0, *scroll_bar_margin),
                );

                let original_style = (*ui.ctx().style()).clone();
                let mut scrollbar_style = original_style.clone();
                scrollbar_style.visuals.extreme_bg_color = theme.scroll_background.into();
                for widget in [
                    &mut scrollbar_style.visuals.widgets.inactive,
                    &mut scrollbar_style.visuals.widgets.hovered,
                    &mut scrollbar_style.visuals.widgets.active,
                ] {
                    widget.fg_stroke.color = theme.scroll_handle.into();
                }
                ui.set_style(scrollbar_style);

                let mut scroll_area = egui::ScrollArea::vertical()
                    .auto_shrink(false)
                    .scroll_bar_rect(scroll_bar_rect);
                if let Some(offset) = scroll_offset {
                    scroll_area = scroll_area.scroll_offset(egui::vec2(0.0, offset));
                }

                scroll_area_output = Some(scroll_area.show(ui, |ui| {
                    ui.set_style(original_style);
                    egui::Frame::new()
                        .inner_margin(egui::Margin::symmetric(padding.x, padding.y))
                        .show(ui, |ui| {
                            if let Err(e) = add_contents(ui) {
                                err = Some(e);
                            }
                        });
                }));
            });

        match err {
            None => Ok(scroll_area_output.unwrap()),
            Some(e) => Err(e),
        }
    }
}
