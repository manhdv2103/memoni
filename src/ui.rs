use std::{
    collections::HashMap,
    ffi::CString,
    fs,
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::{Arc, LazyLock},
};

use anyhow::{Result, anyhow};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontTweak, FullOutput, Painter,
    RawInput, Rect, RichText, Stroke, TextureHandle, Vec2, epaint, scroll_area::ScrollAreaOutput,
};
use fontconfig::Fontconfig;
use image::{GenericImageView, RgbaImage};
use log::{debug, error, info, log_enabled, trace, warn};
use xdg_mime::SharedMimeInfo;

use crate::{
    config::{Config, Dimensions, LayoutConfig},
    freedesktop_cache::get_cached_thumbnail,
    key_action::ScrollAction,
    ordered_hash_map::OrderedHashMap,
    selection::SelectionItem,
    utils::{is_image_mime, is_plaintext_mime, percent_decode, utf16le_to_string},
    widgets::clipboard_button::ClipboardButton,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiFlow {
    TopToBottom,
    BottomToTop,
}

struct ImageInfo {
    r#type: String,
    thumbnail: RgbaImage,
    size: Option<(u32, u32)>,
}

const FALLBACK_IMG_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/images/fallback_image.png"
));
const FALLBACK_FILE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/images/fallback_file.png"
));
const FALLBACK_DIR_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/images/fallback_directory.png"
));
struct Fallback {
    image: RgbaImage,
    file: RgbaImage,
    directory: RgbaImage,
}

const NOTO_SANS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/Noto_Sans/NotoSans-Regular.ttf"
));
const NOTO_SYMBOLS: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/Noto_Sans_Symbols_2/NotoSansSymbols2-Regular.ttf"
));
const NOTO_EMOJI: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fonts/Noto_Emoji/NotoEmoji-Regular.ttf"
));

#[derive(Debug)]
struct ScrollAreaInfo {
    rect: Rect,
    content_rects: HashMap<u64, Rect>,
    offset: f32,
}

#[derive(Debug, PartialEq, Eq, Copy, Clone)]
enum ActiveSource {
    KeyAction,
    Hovering,
}

pub struct Ui<'a> {
    pub egui_ctx: egui::Context,
    config: &'a Config,
    fonts: FontDefinitions,
    item_widget_ids: HashMap<u64, egui::Id>,
    active_source: Option<ActiveSource>,
    scroll_area_info: Option<ScrollAreaInfo>,
    is_initial_run: bool,
    hides_scroll_bar: bool,
    button_widgets: HashMap<u64, ClipboardButton>,
    fallback: Fallback,
}

impl<'a> Ui<'a> {
    pub fn new(config: &'a Config) -> Result<Self> {
        info!("creating egui context");
        let egui_ctx = Self::create_egui_context(config);
        let font = &config.font;
        let mut fonts = FontDefinitions::default();

        info!("setting default fonts");
        fonts.font_data.insert(
            "NotoSans-Regular".to_owned(),
            Arc::new(FontData::from_static(NOTO_SANS)),
        );
        fonts.font_data.insert(
            "NotoEmoji-Regular".to_owned(),
            Arc::new(FontData::from_static(NOTO_EMOJI).tweak(FontTweak {
                scale: 0.81,
                ..Default::default()
            })),
        );
        // Mostly for the newline symbol (⏎)
        fonts.font_data.insert(
            "NotoSansSymbols2-Regular".to_owned(),
            Arc::new(FontData::from_static(NOTO_SYMBOLS).tweak(FontTweak {
                y_offset_factor: 0.175,
                ..Default::default()
            })),
        );

        let mut font_family_names = vec![];

        if !font.families.is_empty() {
            info!("setting custom fonts")
        }
        for (i, font_family) in font.families.iter().enumerate() {
            if let Some(font_path) = Self::find_font(font_family)? {
                debug!("found font family '{font_family}' file: {font_path:?}");
                fonts.font_data.insert(
                    font_family.clone(),
                    Arc::new(FontData::from_owned(fs::read(font_path)?).tweak(FontTweak {
                        y_offset_factor: *font.y_offset_factors.get(i).unwrap_or(&0.0),
                        ..Default::default()
                    })),
                );

                font_family_names.push(font_family.clone());
            } else {
                warn!("font family '{font_family}' not found");
            }
        }

        font_family_names.push("NotoSans-Regular".to_owned());
        font_family_names.push("NotoEmoji-Regular".to_owned());
        font_family_names.push("NotoSansSymbols2-Regular".to_owned());

        fonts
            .families
            .insert(FontFamily::Proportional, font_family_names);
        egui_ctx.set_fonts(fonts.clone());

        debug!("loading fallback images");
        let fallback_img = image::load_from_memory(FALLBACK_IMG_BYTES)?.to_rgba8();
        let fallback_file = image::load_from_memory(FALLBACK_FILE_BYTES)?.to_rgba8();
        let fallback_dir = image::load_from_memory(FALLBACK_DIR_BYTES)?.to_rgba8();

        Ok(Ui {
            egui_ctx,
            config,
            fonts,
            item_widget_ids: HashMap::new(),
            active_source: None,
            scroll_area_info: None,
            is_initial_run: true,
            hides_scroll_bar: config.scroll_bar_auto_hide,
            button_widgets: HashMap::new(),
            fallback: Fallback {
                image: fallback_img,
                file: fallback_file,
                directory: fallback_dir,
            },
        })
    }

    pub fn reset_context(&mut self) {
        info!("recreating egui context");
        let egui_ctx = Self::create_egui_context(self.config);
        egui_ctx.set_fonts(self.fonts.clone());
        self.egui_ctx = egui_ctx;

        debug!("clearing button widgets");
        self.button_widgets.clear();
    }

    fn create_egui_context(config: &Config) -> egui::Context {
        let egui_ctx = egui::Context::default();
        let layout = &config.layout;
        let font = &config.font;
        let theme = &config.theme;

        info!("setting global egui style");
        egui_ctx.style_mut(|style| {
            // style.debug.debug_on_hover = true;
            style.spacing.button_padding = layout.button_padding.into();
            style.spacing.item_spacing = egui::vec2(0.0, layout.button_spacing);
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

            for text_style in [egui::TextStyle::Body, egui::TextStyle::Button] {
                if let Some(font_id) = style.text_styles.get_mut(&text_style) {
                    *font_id = egui::FontId::proportional(font.size);
                }
            }
        });

        egui_ctx
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

        if log_enabled!(log::Level::Trace) {
            trace!("found all fonts with pattern {pat:?}:");
            fonts.print();
        }

        let font = fonts.iter().next();
        Ok(font.and_then(|f| f.filename().map(PathBuf::from)))
    }

    pub fn run(
        &mut self,
        egui_input: RawInput,
        active_id: &mut u64,
        selection_items: &OrderedHashMap<u64, SelectionItem>,
        flow: UiFlow,
        scroll_actions: Vec<ScrollAction>,
        mut on_paste: impl FnMut(&SelectionItem),
    ) -> Result<FullOutput> {
        trace!("painting ui with flow {flow:?}");
        let mut run_error = None;
        let layout = &self.config.layout;
        let prev_active_id = *active_id;

        let items_removed = selection_items.len() < self.item_widget_ids.len();
        let active_item_removed = items_removed && !selection_items.contains_key(active_id);
        let prev_active_rect = self
            .scroll_area_info
            .as_ref()
            .and_then(|info| info.content_rects.get(&prev_active_id).cloned());

        let items_size = selection_items.len();
        for action in scroll_actions {
            let action = if flow == UiFlow::TopToBottom {
                action
            } else {
                action.flipped()
            };

            if let Some(active_idx) = selection_items.iter().position(|(id, _)| *id == *active_id)
                && let Some(scroll_info) = &self.scroll_area_info
            {
                let id_from_idx = |idx| *selection_items.get_by_index(idx).unwrap().0;
                let next_id = match action {
                    ScrollAction::ItemUp => id_from_idx((active_idx + items_size - 1) % items_size),
                    ScrollAction::ItemDown => id_from_idx((active_idx + 1) % items_size),

                    ScrollAction::HalfUp if active_idx == 0 => id_from_idx(items_size - 1),
                    ScrollAction::HalfUp => find_item_at_distance_from(
                        active_idx,
                        -scroll_info.rect.height() / 2.0,
                        selection_items,
                        &scroll_info.content_rects,
                        layout.button_spacing,
                    ),
                    ScrollAction::HalfDown if active_idx == items_size - 1 => id_from_idx(0),
                    ScrollAction::HalfDown => find_item_at_distance_from(
                        active_idx,
                        scroll_info.rect.height() / 2.0,
                        selection_items,
                        &scroll_info.content_rects,
                        layout.button_spacing,
                    ),

                    ScrollAction::PageUp if active_idx == 0 => id_from_idx(items_size - 1),
                    ScrollAction::PageUp => find_item_at_distance_from(
                        active_idx,
                        -scroll_info.rect.height(),
                        selection_items,
                        &scroll_info.content_rects,
                        layout.button_spacing,
                    ),
                    ScrollAction::PageDown if active_idx == items_size - 1 => id_from_idx(0),
                    ScrollAction::PageDown => find_item_at_distance_from(
                        active_idx,
                        scroll_info.rect.height(),
                        selection_items,
                        &scroll_info.content_rects,
                        layout.button_spacing,
                    ),
                };

                *active_id = next_id;
                self.active_source = Some(ActiveSource::KeyAction);
            }
        }

        for ev in &egui_input.events {
            if !self.is_initial_run
                && (matches!(ev, egui::Event::PointerMoved(_))
                    || matches!(ev, egui::Event::MouseWheel { .. }))
            {
                self.active_source = Some(ActiveSource::Hovering);
            }

            // With scroll_bar_auto_hide = true, on window shown, the scroll bar may still be
            // briefly visible, so we hide it before showing the window. This shows the scroll
            // bar back when the pointer starts to move.
            if let egui::Event::PointerMoved(pointer_pos) = ev
                && self.config.scroll_bar_auto_hide
                && self
                    .scroll_area_info
                    .as_ref()
                    .map(|s| s.rect.contains(*pointer_pos))
                    .unwrap_or(false)
            {
                self.hides_scroll_bar = false;
            }
        }

        if flow == UiFlow::BottomToTop && self.is_initial_run {
            self.egui_ctx.request_discard(
                "BottomToTop flow displays new items at the top of the list. When resetting \
                 scroll to the bottom, we need to know the height of newly added items beforehand \
                 to correctly calculate the required scroll offset.",
            );
        } else if items_removed {
            self.egui_ctx
                .request_discard("Recalculate scroll area's content size when items got removed");
        }

        let full_output = self.egui_ctx.run(egui_input, |ctx| {
            // Pick new active item if the current one got removed
            if !ctx.will_discard()
                && active_item_removed
                && self
                    .active_source
                    .is_none_or(|source| source == ActiveSource::KeyAction)
            {
                let nearest_item_id = if let Some(scroll_info) = &self.scroll_area_info
                    && let Some(removed_rect) = prev_active_rect
                {
                    selection_items
                        .iter()
                        .filter_map(|(id, _)| {
                            scroll_info.content_rects.get(id).map(|rect| {
                                (*id, (rect.center().y - removed_rect.center().y).abs())
                            })
                        })
                        .min_by(|(_, dist1), (_, dist2)| dist1.total_cmp(dist2))
                        .map(|(id, _)| id)
                } else {
                    None
                };

                *active_id = nearest_item_id
                    .or_else(|| selection_items.get_by_index(0).map(|(id, _)| *id))
                    .unwrap_or(0);
            }

            // Update active item using hovered item
            if !ctx.will_discard()
                && !self.is_initial_run
                && self
                    .active_source
                    .is_some_and(|source| source == ActiveSource::Hovering)
            {
                let hovered_item = ctx.viewport(|vp| {
                    self.item_widget_ids
                        .iter()
                        .find(|(_, widget_id)| vp.interact_widgets.hovered.contains(widget_id))
                });
                if let Some((&hovered_item_id, _)) = hovered_item {
                    *active_id = hovered_item_id;
                }
            }

            // Active item is scrolled out of view, pick a new one
            if !ctx.will_discard()
                && !self.is_initial_run
                && prev_active_id == *active_id
                && let Some(scroll_info) = &self.scroll_area_info
                && let Some(active_rect) = scroll_info.content_rects.get(active_id)
                && !scroll_info.rect.contains_rect(*active_rect)
            {
                let active_rect_above_view = active_rect.min.y < scroll_info.rect.min.y;
                #[allow(clippy::collapsible_else_if)]
                let near_idx_offset = if flow == UiFlow::TopToBottom {
                    if active_rect_above_view { 0 } else { 1 }
                } else {
                    if active_rect_above_view { 1 } else { 0 }
                };

                let found_idx = selection_items.binary_search_by(|(k, _)| {
                    let rect = scroll_info.content_rects.get(k).unwrap_or(&Rect::ZERO);
                    let order = if active_rect_above_view {
                        rect.min.y.total_cmp(&scroll_info.rect.min.y)
                    } else {
                        rect.max.y.total_cmp(&scroll_info.rect.max.y)
                    };

                    if flow == UiFlow::TopToBottom {
                        order
                    } else {
                        order.reverse()
                    }
                });
                let found_idx = match found_idx {
                    Ok(exact_idx) => Some(exact_idx),
                    Err(near_idx)
                        if near_idx >= near_idx_offset
                            && near_idx - near_idx_offset < selection_items.len() =>
                    {
                        Some(near_idx - near_idx_offset)
                    }
                    Err(_) => None,
                };

                if let Some(found_idx) = found_idx {
                    *active_id = *selection_items.get_by_index(found_idx).unwrap().0;
                }
            }

            let scroll_content_size = self
                .scroll_area_info
                .as_ref()
                .map(|s| {
                    let mut size = 0.0;
                    for (item_id, _) in selection_items {
                        size += s
                            .content_rects
                            .get(item_id)
                            .map(|r| r.height())
                            .unwrap_or(0.0);
                        size += self.config.layout.button_spacing;
                    }
                    size -= self.config.layout.button_spacing;
                    size += (self.config.layout.window_padding.y as f32) * 2.0;
                    size
                })
                .unwrap_or(0.0);
            let content_overflowed = self
                .scroll_area_info
                .as_ref()
                .map(|s| s.rect.height() < scroll_content_size)
                .unwrap_or(false);

            let sets_default_scroll_offset = self.is_initial_run
                // Force items to be at the bottom of the window
                || (flow == UiFlow::BottomToTop && !content_overflowed);
            let sets_active_scroll_offset = prev_active_id != *active_id;
            let next_scroll_offset = if (sets_default_scroll_offset
                || sets_active_scroll_offset
                || items_removed)
                && let Some(scroll_area) = &self.scroll_area_info
            {
                let scroll_rect = scroll_area.rect;
                let scroll_offset = scroll_area.offset;

                if sets_default_scroll_offset {
                    if flow == UiFlow::TopToBottom {
                        Some(0.0)
                    } else {
                        Some(scroll_content_size - scroll_rect.height())
                    }
                } else
                // Force content to be pushed down to fill the removed items' space when at the bottom of the scroll area
                if items_removed
                    && content_overflowed
                    && scroll_offset + scroll_rect.height() > scroll_content_size
                {
                    Some(scroll_content_size - scroll_rect.height())
                } else if let Some(active_item) = selection_items.get(active_id)
                    && let Some(&active_rect) = scroll_area.content_rects.get(&active_item.id)
                    && !scroll_rect.contains_rect(active_rect)
                {
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

            self.item_widget_ids.clear();

            let mut content_sizes = HashMap::new();
            let container_result = Self::container(
                ctx,
                self.config,
                next_scroll_offset,
                self.hides_scroll_bar,
                |ui| {
                    if selection_items.is_empty() {
                        ui.centered_and_justified(|ui| {
                            ui.add(egui::Label::new("Your clipboard history will appear here."))
                        });
                        return Ok(());
                    }

                    let layout_reversed = flow == UiFlow::BottomToTop;
                    let item_it: Box<dyn Iterator<Item = _>> = if layout_reversed {
                        Box::new(selection_items.iter().rev())
                    } else {
                        Box::new(selection_items.iter())
                    };

                    for (&id, item) in item_it {
                        let is_active = id == *active_id;

                        let btn = ui.add(
                            self.button_widgets
                                .get(&item.id)
                                .ok_or_else(|| {
                                    anyhow!("missing button widget for item {}", item.id)
                                })?
                                .clone()
                                .is_active(is_active),
                        );

                        self.item_widget_ids.insert(id, btn.id);
                        content_sizes.insert(item.id, btn.rect);

                        if btn.clicked() {
                            on_paste(item);
                        }
                    }

                    Ok(())
                },
            );

            match container_result {
                Ok(scroll_area_output) => {
                    self.scroll_area_info = Some(ScrollAreaInfo {
                        rect: scroll_area_output.inner_rect,
                        content_rects: content_sizes,
                        offset: scroll_area_output.state.offset[1],
                    });
                }
                Err(err) => run_error = Some(err),
            }
        });

        self.is_initial_run = false;

        match run_error {
            None => Ok(full_output),
            Some(err) => Err(err),
        }
    }

    fn container(
        ctx: &egui::Context,
        config: &Config,
        scroll_offset: Option<f32>,
        hides_scroll_bar: bool,
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
                if config.show_ribbon {
                    Self::draw_ribbon(
                        ui.painter(),
                        &ui.min_rect(),
                        config.layout.ribbon_size,
                        config.theme.ribbon,
                    );
                }

                let scroll_bar_rect = egui::Rect::from_min_max(
                    ui.min_rect().min + egui::vec2(0.0, scroll_bar_margin),
                    ui.max_rect().max - egui::vec2(0.0, scroll_bar_margin),
                );

                let original_style = (*ui.ctx().style()).clone();
                let mut scrollbar_style = original_style.clone();
                scrollbar_style.visuals.extreme_bg_color = theme.scroll_background.into();

                if hides_scroll_bar {
                    scrollbar_style.spacing.scroll.dormant_background_opacity = 0.0;
                    scrollbar_style.spacing.scroll.dormant_handle_opacity = 0.0;
                    scrollbar_style.spacing.scroll.active_background_opacity = 0.0;
                    scrollbar_style.spacing.scroll.active_handle_opacity = 0.0;
                } else if !config.scroll_bar_auto_hide {
                    scrollbar_style.spacing.scroll.dormant_background_opacity =
                        scrollbar_style.spacing.scroll.active_background_opacity;
                    scrollbar_style.spacing.scroll.dormant_handle_opacity =
                        scrollbar_style.spacing.scroll.active_handle_opacity;
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

    fn draw_ribbon(painter: &Painter, container_rect: &Rect, size: f32, color: impl Into<Color32>) {
        let mut points = [
            egui::pos2(-size, 0.0),
            egui::pos2(0.0, 0.0),
            egui::pos2(0.0, size),
        ];
        for p in &mut points {
            p.x += container_rect.width();
        }

        painter.add(epaint::Shape::convex_polygon(
            points.to_vec(),
            color,
            Stroke::NONE,
        ));
    }

    pub fn reset(&mut self) {
        info!("resetting ui states");
        self.active_source = None;
        self.is_initial_run = true;
        self.hides_scroll_bar = self.config.scroll_bar_auto_hide;
    }

    pub fn build_button_widget(&mut self, item: &SelectionItem) -> Result<()> {
        trace!("building button widget for item {}", item.id);
        let Ui {
            egui_ctx: ctx,
            config,
            fallback,
            ..
        } = self;

        let mut text_content = None;
        let mut img_info = None;
        let mut img_metadata = None;
        let mut files = None;
        for (mime, data) in &item.data {
            if is_plaintext_mime(mime) {
                text_content = Some(str::from_utf8(data)?);
            } else if is_image_mime(mime) {
                let img_type = mime.split(['/', '+']).nth(1).unwrap_or(mime).to_uppercase();
                let img = if img_type == "SVG" {
                    load_svg(data, config.layout.preview_size.into())
                } else {
                    image::load_from_memory(data)
                        .map(|i| (i.to_rgba8(), i.dimensions()))
                        .map_err(anyhow::Error::from)
                };

                img_info = Some(match img {
                    Ok((img, size)) => {
                        let thumbnail = create_thumbnail(&img, config.layout.preview_size.into());
                        ImageInfo {
                            r#type: img_type,
                            size: Some(size),
                            thumbnail,
                        }
                    }
                    Err(err) => {
                        error!(
                            "failed to load image with mime {mime} of item {}: {err}",
                            item.id
                        );
                        ImageInfo {
                            r#type: img_type,
                            size: None,
                            thumbnail: fallback.image.clone(),
                        }
                    }
                });
            } else if mime == "text/x-moz-url" {
                // Firefox encodes data with UTF-16
                // https://stackoverflow.com/a/51581772
                let data = utf16le_to_string(data);
                img_metadata = Some(
                    data.split_once('\n')
                        .map(|(s, a)| (s.to_string(), a.to_string()))
                        .unwrap_or((data, "".to_string())),
                );
            } else if mime == "text/uri-list" && files.is_none() {
                let uris = str::from_utf8(data)?
                    .lines()
                    // text/uri-list can contain comment (based on RFC 2483)
                    .filter(|l| !l.is_empty() && !l.starts_with("#"))
                    .collect::<Vec<_>>();
                if uris.len() == uris.iter().filter(|u| u.starts_with("file://")).count() {
                    files = Some((None, uris));
                }
            } else if mime == "x-special/gnome-copied-files" {
                let mut file_iter = str::from_utf8(data)?.lines();
                let action = file_iter.next();
                files = Some((action, file_iter.collect()));
            }
        }

        let mut btn = ClipboardButton::default()
            .underline_offset(config.font.underline_offset)
            .with_preview_padding(config.layout.button_with_preview_padding);

        if let Some((action, file_uris)) = files {
            let file_paths = file_uris
                .iter()
                .map(|u| {
                    str::from_utf8(&percent_decode(&u.as_bytes()["file://".len()..]))
                        .unwrap()
                        .to_owned()
                })
                .collect::<Vec<_>>();
            let mut path_iter = file_paths.iter();
            if let Some(path) = path_iter.next() {
                btn = btn.append_label(format_path_str(path));
            }
            if let Some(path) = path_iter.next() {
                btn = btn.append_label(format_path_str(path));
            }
            let more_count = path_iter.count();

            let mut sublabel_text = "".to_owned();
            if let Some(action) = action {
                sublabel_text.push_str(action);
            }

            if more_count > 0 {
                if !sublabel_text.is_empty() {
                    sublabel_text.push_str(" | ");
                }
                sublabel_text.push_str(&format!("+{more_count} MORE..."));
            }

            if !sublabel_text.is_empty() {
                btn = btn.sublabel(
                    RichText::new(sublabel_text.to_uppercase())
                        .size(config.font.secondary_size)
                        .color(config.theme.muted_foreground),
                )
            }

            let thumbnail = create_files_thumbnail(
                &file_paths,
                config.layout.preview_size,
                &fallback.file,
                &fallback.directory,
            );
            let texture = load_texture(ctx, item.id, &thumbnail);
            btn = btn.preview(texture, config.layout.preview_size);
        } else if let Some(ImageInfo {
            r#type,
            size,
            thumbnail,
        }) = img_info
        {
            let texture = load_texture(ctx, item.id, &thumbnail);
            let sublabel_text = if let Some(size) = size {
                format!("{} [{}x{}]", r#type, size.0, size.1)
            } else {
                format!("{} [?x?]", r#type)
            };

            btn = btn
                .preview(texture, config.layout.preview_size)
                .sublabel(
                    RichText::new(sublabel_text)
                        .size(config.font.secondary_size)
                        .color(config.theme.muted_foreground),
                )
                .preview_background(config.theme.preview_background);

            if let Some((src, alt)) = img_metadata {
                if !alt.is_empty() {
                    btn = btn.label(normalize_display_string(&alt));
                }
                btn = btn.preview_source(&src);
            }
        } else if let Some(text) = text_content {
            btn = btn.label(normalize_display_string(text));
        } else {
            btn = btn.label(RichText::new("[unknown]").color(config.theme.muted_foreground));
        }

        self.button_widgets.insert(item.id, btn);
        Ok(())
    }

    pub fn remove_button_widgets<I: IntoIterator<Item = SelectionItem>>(
        &mut self,
        removed_items: I,
    ) {
        for item in removed_items {
            trace!("removing button widget for item {}", item.id);
            self.button_widgets.remove(&item.id);
        }
    }
}

fn find_item_at_distance_from(
    from_idx: usize,
    distance: f32,
    items: &OrderedHashMap<u64, SelectionItem>,
    item_rects: &HashMap<u64, Rect>,
    item_gap: f32,
) -> u64 {
    let items_size = items.len();
    assert!(items_size > 0);
    assert!(from_idx < items_size);

    #[derive(PartialEq)]
    enum Dir {
        Up,
        Down,
    }

    let target_dist = distance.abs();
    let (start, end, dir) = if distance >= 0.0 {
        if from_idx == items_size - 1 {
            return *items.get_by_index(from_idx).unwrap().0;
        }
        (from_idx + 1, items.len() - 1, Dir::Down)
    } else {
        if from_idx == 0 {
            return *items.get_by_index(from_idx).unwrap().0;
        }
        (from_idx - 1, 0, Dir::Up)
    };

    let mut to_idx = start;
    let mut total_dist = 0.0;
    loop {
        if let Some(rect) = item_rects.get(items.get_by_index(to_idx).unwrap().0) {
            total_dist += rect.height() + item_gap;
            if total_dist >= target_dist {
                break;
            }
        }

        if to_idx == end {
            break;
        };
        to_idx = if dir == Dir::Down {
            to_idx + 1
        } else {
            to_idx - 1
        };
    }

    *items.get_by_index(to_idx).unwrap().0
}

fn create_files_thumbnail(
    files: &[String],
    size: Dimensions,
    fallback_file: &RgbaImage,
    fallback_dir: &RgbaImage,
) -> RgbaImage {
    let mut thumbnail = RgbaImage::from_pixel(
        size.width.into(),
        size.height.into(),
        image::Rgba([0, 0, 0, 0]),
    );

    let display_count = files.len().min(4);
    if display_count == 0 {
        return thumbnail;
    }

    // relative coordinates of each file thumbnail inside the thumbnail
    static TEMPLATES: &[&[&[f32; 4]]] = &[
        &[&[0.1, 0.1, 0.9, 0.9]],
        &[&[0.1, 0.1, 0.6, 0.6], &[0.4, 0.4, 0.9, 0.9]],
        &[
            &[0.1, 0.1, 0.55, 0.55],
            &[0.45, 0.2, 0.9, 0.65],
            &[0.225, 0.45, 0.675, 0.9],
        ],
        &[
            &[0.15, 0.1, 0.6, 0.55],
            &[0.45, 0.15, 0.9, 0.6],
            &[0.1, 0.4, 0.55, 0.85],
            &[0.4, 0.45, 0.85, 0.9],
        ],
    ];
    let template = TEMPLATES[display_count - 1];

    for i in 0..display_count {
        let file = &files[i];
        let is_dir = Path::new(file).is_dir();
        let file_thumb_temp = template[i];
        let coord = &[
            (file_thumb_temp[0] * size.width as f32).round() as u16,
            (file_thumb_temp[1] * size.height as f32).round() as u16,
            (file_thumb_temp[2] * size.width as f32).round() as u16,
            (file_thumb_temp[3] * size.height as f32).round() as u16,
        ];
        let size = Vec2::new((coord[2] - coord[0]).into(), (coord[3] - coord[1]).into());

        let file_thumb = get_file_thumbnail(file, size, is_dir).unwrap_or_else(|e| {
            error!("failed to get file thumbnail for {file}: {e}");
            None
        });
        let fallback = if is_dir { fallback_dir } else { fallback_file };
        let file_thumb = file_thumb.as_ref().unwrap_or(fallback);
        let scaled_file_thumb = create_thumbnail(file_thumb, size);
        image::imageops::overlay(
            &mut thumbnail,
            &scaled_file_thumb,
            coord[0].into(),
            coord[1].into(),
        );
    }

    thumbnail
}

fn get_file_thumbnail<P: AsRef<Path>>(
    file: P,
    size_hint: Vec2,
    is_dir: bool,
) -> Result<Option<RgbaImage>> {
    let thumb_path = if is_dir {
        freedesktop_icon::get_icon("folder")
    } else {
        get_cached_thumbnail(&file)
            .unwrap_or_else(|e| {
                warn!(
                    "failed to get cached thumbnail for {:?}: {e}",
                    file.as_ref()
                );
                None
            })
            .or_else(|| {
                get_file_icon_path(&file).unwrap_or_else(|e| {
                    warn!("failed to get icon for {:?}: {e}", file.as_ref());
                    None
                })
            })
    };
    let Some(path) = thumb_path else {
        return Ok(None);
    };

    if let Some(ext) = path.extension()
        && (ext == "png" || ext == "svg")
    {
        let data = fs::read(&path)?;
        if ext == "png" {
            Ok(Some(image::load_from_memory(&data)?.to_rgba8()))
        } else {
            Ok(Some(load_svg(&data, size_hint)?.0))
        }
    } else {
        warn!("unsupported thumbnail file type, expected png or svg: {path:?}");
        Ok(None)
    }
}

fn get_file_icon_path<P: AsRef<Path>>(file: P) -> Result<Option<PathBuf>> {
    static SMI: LazyLock<SharedMimeInfo> = LazyLock::new(SharedMimeInfo::new);

    let data_mime = SMI
        .get_mime_type_for_data(&fs::read(&file)?)
        .map(|(mime, _)| mime);
    let ext_mime = file
        .as_ref()
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| SMI.get_mime_types_from_file_name(name).first().cloned());

    let mime = if let Some(data_mime) = data_mime {
        if let Some(ext_mime) = ext_mime
            && SMI.mime_type_subclass(&ext_mime, &data_mime)
        {
            ext_mime
        } else {
            data_mime
        }
    } else if let Some(ext_mime) = ext_mime {
        ext_mime
    } else {
        mime::Mime::from_str("application/x-generic")?
    };

    for icon_name in SMI.lookup_icon_names(&mime) {
        if let Some(icon) = freedesktop_icon::get_icon(&icon_name) {
            return Ok(Some(icon));
        }
    }

    debug!(
        "icon for {:?} with mime '{}' not found",
        file.as_ref(),
        mime
    );
    Ok(None)
}

fn create_thumbnail(image: &RgbaImage, size: Vec2) -> RgbaImage {
    let orig_w = image.width() as f32;
    let orig_h = image.height() as f32;
    let scale = (size.x / orig_w).min(size.y / orig_h);
    let thumb_w = (orig_w * scale).round() as u32;
    let thumb_h = (orig_h * scale).round() as u32;

    let scaled = image::imageops::thumbnail(image, thumb_w, thumb_h);

    let mut thumbnail =
        RgbaImage::from_pixel(size.x as u32, size.y as u32, image::Rgba([0, 0, 0, 0]));
    let (w, h) = scaled.dimensions();
    let x_offset = (size.x as u32 - w) / 2;
    let y_offset = (size.y as u32 - h) / 2;
    for y in 0..h {
        for x in 0..w {
            let px = scaled.get_pixel(x, y);
            thumbnail.put_pixel(x + x_offset, y + y_offset, *px);
        }
    }

    thumbnail
}

fn load_texture(ctx: &egui::Context, id: u64, img: &RgbaImage) -> TextureHandle {
    let thumb_size = [img.width() as usize, img.height() as usize];
    ctx.load_texture(
        id.to_string(),
        egui::ColorImage::from_rgba_unmultiplied(thumb_size, img.as_flat_samples().as_slice()),
        Default::default(),
    )
}

pub fn load_svg(svg_bytes: &[u8], size_hint: Vec2) -> Result<(RgbaImage, (u32, u32))> {
    use resvg::{
        tiny_skia::Pixmap,
        usvg::{Options, Transform, Tree},
    };

    let rtree = Tree::from_data(svg_bytes, &Options::default())?;
    let source_size = Vec2::new(rtree.size().width(), rtree.size().height());

    let mut scaled_size = source_size;
    scaled_size *= size_hint.x / source_size.x;
    if scaled_size.y > size_hint.y {
        scaled_size *= size_hint.y / scaled_size.y;
    }
    let scaled_size = scaled_size.round();
    let (w, h) = (scaled_size.x as u32, scaled_size.y as u32);

    let mut pixmap =
        Pixmap::new(w, h).ok_or_else(|| anyhow!("failed to create SVG Pixmap of size {w}x{h}"))?;

    resvg::render(
        &rtree,
        Transform::from_scale(w as f32 / source_size.x, h as f32 / source_size.y),
        &mut pixmap.as_mut(),
    );

    Ok((
        RgbaImage::from_raw(
            w,
            h,
            pixmap
                .pixels()
                .iter()
                .map(|p| p.demultiply())
                .flat_map(|p| [p.red(), p.green(), p.blue(), p.alpha()])
                .collect(),
        )
        .ok_or_else(|| anyhow!("failed to create RgbaImage"))?,
        (w, h),
    ))
}

fn normalize_display_string(s: &str) -> String {
    let mut res = String::with_capacity(s.len());
    for (i, c) in s.chars().enumerate() {
        // Very very long string causes egui to choke on first render, even when we only display it on a single line
        if i == 10_000 && i < s.len() - 1 {
            res.push('…');
            break;
        }

        match c {
            '\r' => {}
            '\n' => res.push('⏎'),
            _ => res.push(c),
        }
    }

    res
}

fn format_path_str(path: &str) -> String {
    let home = dirs::home_dir();
    let mut s = String::with_capacity(path.len());

    if let Some(home) = home
        && let Some(home_str) = home.to_str()
        && let Some(stripped) = path.strip_prefix(home_str)
    {
        s.push('~');
        s.push_str(stripped);
    } else {
        s.push_str(path);
    }

    let p = Path::new(path);
    if p.is_dir() && !s.ends_with('/') {
        s.push('/');
    }

    s
}
