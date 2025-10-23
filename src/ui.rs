use std::{
    collections::{HashMap, VecDeque},
    ffi::CString,
    fs,
    path::{Path, PathBuf},
    str::FromStr as _,
    sync::{Arc, LazyLock},
};

use anyhow::{Result, anyhow};
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontTweak, FullOutput, RawInput,
    RichText, Stroke, TextureHandle, Vec2, scroll_area::ScrollAreaOutput,
};
use fontconfig::Fontconfig;
use image::{GenericImageView, RgbaImage};
use xdg_mime::SharedMimeInfo;

use crate::{
    config::{Config, Dimensions, LayoutConfig},
    freedesktop_cache::get_cached_thumbnail,
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
    "/assets/fallback_image.png"
));
const FALLBACK_FILE_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fallback_file.png"
));
const FALLBACK_DIR_BYTES: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/assets/fallback_directory.png"
));
struct Fallback {
    image: RgbaImage,
    file: RgbaImage,
    directory: RgbaImage,
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
    button_widgets: HashMap<u64, ClipboardButton>,
    fallback: Fallback,
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
                        y_offset_factor: font.y_offset_factor,
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

        let fallback_img = image::load_from_memory(FALLBACK_IMG_BYTES)?.to_rgba8();
        let fallback_file = image::load_from_memory(FALLBACK_FILE_BYTES)?.to_rgba8();
        let fallback_dir = image::load_from_memory(FALLBACK_DIR_BYTES)?.to_rgba8();

        Ok(Ui {
            egui_ctx,
            config,
            item_ids: vec![],
            hovered_idx: None,
            active_idx: 0,
            scroll_area_output: None,
            is_initial_run: true,
            shows_scroll_bar: false,
            button_widgets: HashMap::new(),
            fallback: Fallback {
                image: fallback_img,
                file: fallback_file,
                directory: fallback_dir,
            },
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
                    let focus_step = match key {
                        egui::Key::ArrowUp | egui::Key::K => -1,
                        egui::Key::P if modifiers.ctrl => -1,
                        egui::Key::Tab if modifiers.shift => -1,
                        egui::Key::ArrowDown | egui::Key::J => 1,
                        egui::Key::N if modifiers.ctrl => 1,
                        egui::Key::Tab => 1,

                        // TODO: proper half-page/full-page step (based on window and item sizes)
                        egui::Key::D if modifiers.ctrl => 5,
                        egui::Key::U if modifiers.ctrl => -5,
                        egui::Key::F if modifiers.ctrl => 10,
                        egui::Key::B if modifiers.ctrl => -10,
                        _ => 0,
                    } * if flow == UiFlow::BottomToTop { -1 } else { 1 };
                    if focus_step != 0 && !self.item_ids.is_empty() {
                        // Reduce step size to 1 to help reorient user when wrapping while
                        // doing half-page/full-page scroll
                        let focus_step = if (self.active_idx == 0 && focus_step < 0)
                            || (self.active_idx == self.item_ids.len() - 1 && focus_step > 0)
                        {
                            if focus_step > 0 { 1 } else { -1 }
                        } else {
                            focus_step
                        };

                        let new_active_idx = self.active_idx as isize + focus_step;

                        // Snap to start/end of the list if overshooting while
                        // doing half-page/full-page scroll
                        self.active_idx = if (self.active_idx == 0
                            && new_active_idx < self.active_idx as isize)
                            || (self.active_idx == self.item_ids.len() - 1
                                && new_active_idx > self.active_idx as isize)
                        {
                            new_active_idx.rem_euclid(self.item_ids.len() as isize) as usize
                        } else {
                            new_active_idx.clamp(0, self.item_ids.len() as isize - 1) as usize
                        };

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
                .map(|s| s.inner_rect.height() < s.content_size[1])
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

                        let btn = ui.add(
                            self.button_widgets
                                .get(&item.id)
                                .ok_or_else(|| anyhow!("missing button widget for item {}", item.id))?
                                .clone()
                                .is_active(is_active)
                        );

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

    pub fn build_button_widget(&mut self, item: &SelectionItem) -> Result<()> {
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
                        eprintln!("error: image with mime {}: {}", mime, err);
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
            if let Some(file) = path_iter.next() {
                btn = btn.append_label(format_path_str(file));
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
                sublabel_text.push_str(&format!("+{} MORE...", more_count));
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
            eprintln!("error: get file thumbnail for {}: {e}", file);
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
                eprintln!(
                    "warning: cannot get cached thumbnail for {}: {e}",
                    file.as_ref().to_string_lossy(),
                );
                None
            })
            .or_else(|| {
                get_file_icon_path(&file).unwrap_or_else(|e| {
                    eprintln!(
                        "warning: cannot get icon for {}: {e}",
                        file.as_ref().to_string_lossy(),
                    );
                    None
                })
            })
    };

    if let Some(path) = thumb_path
        && let Some(ext) = path.extension()
        && (ext == "png" || ext == "svg")
    {
        let data = fs::read(&path)?;
        if ext == "png" {
            Ok(Some(image::load_from_memory(&data)?.to_rgba8()))
        } else {
            Ok(Some(load_svg(&data, size_hint)?.0))
        }
    } else {
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
