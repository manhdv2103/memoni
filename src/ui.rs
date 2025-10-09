use std::{
    collections::{HashMap, VecDeque},
    ffi::CString,
    fs,
    path::PathBuf,
    sync::Arc,
};

use anyhow::{Result, anyhow};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontTweak, FullOutput, RawInput,
    RichText, Stroke, scroll_area::ScrollAreaOutput,
};
use fontconfig::Fontconfig;
use image::RgbaImage;

use crate::{
    config::{Config, Dimensions, LayoutConfig},
    selection::SelectionItem,
    utils::{is_image_mime, is_plaintext_mime},
    widgets::clipboard_button::ClipboardButton,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiFlow {
    TopToBottom,
    BottomToTop,
}

struct ImageInfo {
    thumbnail: RgbaImage,
    size: (u32, u32),
}

pub struct Ui<'a> {
    pub egui_ctx: egui::Context,
    config: &'a Config,
    item_ids: Vec<egui::Id>,
    hovered_idx: Option<usize>,
    active_idx: usize,
    scroll_area_output: Option<ScrollAreaOutput<()>>,
    is_initial_run: bool,
    shows_scroll_bar: bool,
    img_info_cache: HashMap<u64, ImageInfo>,
}

impl<'a> Ui<'a> {
    pub fn new(config: &'a Config) -> Result<Self> {
        let egui_ctx = egui::Context::default();
        let layout = &config.layout;
        let font = &config.font;
        let theme = &config.theme;

        egui_ctx.style_mut(|style| {
            // style.debug.debug_on_hover = true;
            style.spacing.button_padding = layout.button_padding.into();
            style.spacing.item_spacing = layout.item_spacing.into();
            style.interaction.selectable_labels = false;

            style.visuals.override_text_color = Some(theme.foreground.into());
            for widget in [
                &mut style.visuals.widgets.inactive,
                &mut style.visuals.widgets.hovered,
                &mut style.visuals.widgets.active,
            ] {
                widget.fg_stroke.color = theme.foreground.into();
                widget.weak_bg_fill = theme.button_background.into();
                widget.corner_radius = CornerRadius::same(layout.button_corner_radius);
                widget.bg_stroke = Stroke::NONE;
                widget.expansion = 0.0;
            }
            style.visuals.widgets.active.weak_bg_fill = theme.button_active_background.into();

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
            is_initial_run: true,
            shows_scroll_bar: false,
            img_info_cache: HashMap::new(),
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

    pub fn run(
        &mut self,
        mut egui_input: RawInput,
        selection_items: &VecDeque<SelectionItem>,
        flow: UiFlow,
        mut on_paste: impl FnMut(&SelectionItem),
    ) -> Result<FullOutput> {
        let mut run_error = None;
        let layout = &self.config.layout;

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
                        self.active_idx = (self.active_idx as isize
                            + focus_direction * if flow == UiFlow::BottomToTop { -1 } else { 1 })
                        .rem_euclid(self.item_ids.len() as isize)
                            as usize;
                        move_by_key = true;
                    }

                    if matches!(key, egui::Key::Enter | egui::Key::Space)
                        && let Some(item) = selection_items.get(self.active_idx)
                    {
                        on_paste(item);
                    }
                }
                return false;
            }

            if let egui::Event::PointerMoved(pointer_pos) = ev {
                pointer_moved = true;
                if self
                    .scroll_area_output
                    .as_ref()
                    .map(|s| s.inner_rect.contains(*pointer_pos))
                    .unwrap_or(false)
                {
                    self.shows_scroll_bar = true;
                }
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

            let content_overflowed = self
                .scroll_area_output
                .as_ref()
                .map(|s| (s.inner_rect.height() - s.content_size[1]) < 0.0)
                .unwrap_or(false);

            let set_default_scroll_offset = self.is_initial_run
                // offset items to the bottom
                || (flow == UiFlow::BottomToTop && !content_overflowed);
            let set_active_scroll_offset = prev_active_idx != self.active_idx;
            let next_scroll_offset = if (set_default_scroll_offset || set_active_scroll_offset)
                && let Some(scroll_area) = &self.scroll_area_output
                && let Some(&active_id) = self.item_ids.get(self.active_idx)
                && let Some(active_rect) =
                    ctx.viewport(|vp| vp.prev_pass.widgets.get(active_id).map(|w| w.rect))
            {
                let scroll_rect = scroll_area.inner_rect;
                let scroll_content_size = scroll_area.content_size[1];
                let scroll_offset = scroll_area.state.offset[1];

                if set_default_scroll_offset {
                    if flow == UiFlow::TopToBottom {
                        Some(0.0)
                    } else {
                        Some(scroll_content_size - scroll_rect.height())
                    }
                } else if !scroll_rect.contains_rect(active_rect) {
                    let padding = layout.window_padding.y as f32;
                    if active_rect.top() < scroll_rect.top() + padding {
                        Some(scroll_offset + active_rect.top() - padding)
                    } else {
                        Some(scroll_offset + active_rect.bottom() - scroll_rect.height() + padding)
                    }
                } else {
                    None
                }
            } else {
                None
            };

            self.item_ids.clear();

            let container_result = Self::container(ctx, self.config, next_scroll_offset,
                self.shows_scroll_bar, |ui| {

                    if selection_items.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.add(egui::Label::new("Your clipboard history will appear here."))
                        });
                        return Ok(());
                    }

                    let layout_reversed = flow == UiFlow::BottomToTop;
                    let item_it: Box<dyn Iterator<Item = _>> = if layout_reversed {
                        Box::new(selection_items.iter().enumerate().rev())
                    } else {
                        Box::new(selection_items.iter().enumerate())
                    };

                    for (i, item) in item_it {
                        let is_active = i == self.active_idx;

                        let btn = ui.add(Self::render_item(
                            ctx, self.config, item, is_active, &mut self.img_info_cache)?);
                        if layout_reversed {
                            self.item_ids.insert(0, btn.id);
                        } else {
                            self.item_ids.push(btn.id);
                        }

                        if btn.clicked() {
                            on_paste(item);
                        }
                    }

                    Ok(())
                });

            if flow == UiFlow::BottomToTop && self.is_initial_run {
                self.egui_ctx.request_discard(
                    "BottomToTop flow displays new items at the top of the list. When resetting \
                     scroll to the bottom, we need to know the height of newly added items beforehand \
                     to correctly calculate the required scroll offset.",
                );
            }

            match container_result {
                Ok(scroll_area_output) => self.scroll_area_output = Some(scroll_area_output),
                Err(err) => run_error = Some(err),
            }
        });

        self.is_initial_run = false;

        match run_error {
            None => Ok(full_output),
            Some(err) => Err(err),
        }
    }

    pub fn reset(&mut self) {
        self.active_idx = 0;
        self.hovered_idx = None;
        self.is_initial_run = true;
        self.shows_scroll_bar = false;
    }

    fn container(
        ctx: &egui::Context,
        config: &Config,
        scroll_offset: Option<f32>,
        shows_scroll_bar: bool,
        add_contents: impl FnOnce(&mut egui::Ui) -> Result<()>,
    ) -> Result<ScrollAreaOutput<()>> {
        let LayoutConfig {
            window_padding: padding,
            scroll_bar_margin,
            ..
        } = config.layout;
        let theme = &config.theme;
        let mut scroll_area_output = None;
        let mut err: Option<anyhow::Error> = None;

        egui::CentralPanel::default()
            .frame(egui::Frame::new())
            .show(ctx, |ui| {
                let scroll_bar_rect = egui::Rect::from_min_max(
                    ui.min_rect().min + egui::vec2(0.0, scroll_bar_margin),
                    ui.max_rect().max - egui::vec2(0.0, scroll_bar_margin),
                );

                let original_style = (*ui.ctx().style()).clone();
                let mut scrollbar_style = original_style.clone();
                scrollbar_style.visuals.extreme_bg_color = theme.scroll_background.into();
                for widget in [
                    &mut scrollbar_style.visuals.widgets.inactive,
                    &mut scrollbar_style.visuals.widgets.hovered,
                    &mut scrollbar_style.visuals.widgets.active,
                ] {
                    // Cannot use scroll_bar_visibility as the scroll bar has fading animation when hiding
                    widget.fg_stroke.color = if shows_scroll_bar {
                        theme.scroll_handle.into()
                    } else {
                        Color32::TRANSPARENT
                    };
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

    fn render_item(
        ctx: &egui::Context,
        config: &Config,
        item: &SelectionItem,
        is_active: bool,
        img_info_cache: &mut HashMap<u64, ImageInfo>,
    ) -> Result<ClipboardButton> {
        let mut text_content = None;
        let mut img_info = None;
        let mut img_metadata = None;
        for (mime, data) in &item.data {
            if is_plaintext_mime(mime) {
                text_content = Some(str::from_utf8(data)?);
            } else if is_image_mime(mime) {
                img_info = Some(img_info_cache.entry(item.id).or_insert_with(|| {
                    let image = image::load_from_memory(data).unwrap().to_rgba8();
                    let thumbnail = Self::create_thumbnail(&image, config.layout.preview_size);
                    ImageInfo {
                        size: image.dimensions(),
                        thumbnail,
                    }
                }));
            } else if mime == "text/x-moz-url" {
                // Firefox encodes data with UTF-16
                // https://stackoverflow.com/a/51581772
                let data = utf16le_to_string(data);
                img_metadata = Some(
                    data.split_once('\n')
                        .map(|(s, a)| (s.to_string(), a.to_string()))
                        .unwrap_or((data, "".to_string())),
                );
            }
        }

        let mut btn = ClipboardButton::default()
            .is_active(is_active)
            .underline_offset(config.font.underline_offset);

        if let Some(ImageInfo {
            size,
            thumbnail: thumb,
        }) = img_info
        {
            let thumb_size = [thumb.width() as usize, thumb.height() as usize];
            let texture = ctx.load_texture(
                item.id.to_string(),
                egui::ColorImage::from_rgba_unmultiplied(
                    thumb_size,
                    thumb.as_flat_samples().as_slice(),
                ),
                Default::default(),
            );

            btn = btn
                .image(texture, config.layout.preview_size)
                .sublabel(
                    RichText::new(format!("[{}x{}]", size.0, size.1))
                        .size(config.font.secondary_size)
                        .color(config.theme.muted_foreground),
                )
                .with_preview_padding(config.layout.button_with_preview_padding);

            if let Some((src, alt)) = img_metadata {
                if !alt.is_empty() {
                    btn = btn.label(alt);
                }
                btn = btn.image_source(&src);
            }
        } else if let Some(text) = text_content {
            btn = btn.label(text);
        } else {
            btn = btn.label(RichText::new("[unknown]").color(config.theme.muted_foreground));
        }

        Ok(btn)
    }

    fn create_thumbnail(image: &RgbaImage, size: Dimensions) -> RgbaImage {
        let orig_w = image.width() as f32;
        let orig_h = image.height() as f32;
        let scale = (size.width as f32 / orig_w).min(size.height as f32 / orig_h);
        let thumb_w = (orig_w * scale).round() as u32;
        let thumb_h = (orig_h * scale).round() as u32;

        let scaled = image::imageops::thumbnail(image, thumb_w, thumb_h);

        let mut thumbnail = RgbaImage::from_pixel(
            size.width.into(),
            size.height.into(),
            image::Rgba([0, 0, 0, 0]),
        );
        let (w, h) = scaled.dimensions();
        let x_offset = (size.width as u32 - w) / 2;
        let y_offset = (size.height as u32 - h) / 2;
        for y in 0..h {
            for x in 0..w {
                let px = scaled.get_pixel(x, y);
                thumbnail.put_pixel(x + x_offset, y + y_offset, *px);
            }
        }

        thumbnail
    }
}

fn utf16le_to_string(bytes: &[u8]) -> String {
    assert!(bytes.len() % 2 == 0);
    let u16_slice: &[u16] =
        unsafe { std::slice::from_raw_parts(bytes.as_ptr() as *const u16, bytes.len() / 2) };
    String::from_utf16_lossy(u16_slice)
}
