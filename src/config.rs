use anyhow::{Context, Result};
use egui::Color32;
use egui::ecolor::ParseHexColorError;
use serde::Deserialize;
use serde_with::{DisplayFromStr, FromInto, OneOrMany, serde_as};
use std::collections::HashMap;
use std::fmt::{self, Display, Formatter};
use std::fs;
use std::ops::Deref;
use std::path::PathBuf;
use std::str::FromStr;
use xkeysym::Keysym;

#[serde_as]
#[derive(Deserialize, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    #[serde_as(as = "HashMap<_, OneOrMany<_>>")]
    pub paste_bindings: HashMap<String, Vec<Binding>>,
    pub layout: LayoutConfig,
    pub font: FontConfig,
    pub theme: ThemeConfig,
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct LayoutConfig {
    pub window_dimensions: Dimensions,
    pub window_padding: XY<i8>,
    pub button_padding: XY<f32>,
    pub button_with_preview_padding: XY<f32>,
    pub button_corner_radius: u8,
    pub item_spacing: XY<f32>,
    pub scroll_bar_margin: f32,
    pub pointer_gap: i32,
    pub screen_edge_gap: i32,
    pub preview_size: Dimensions,
}

impl Default for LayoutConfig {
    fn default() -> Self {
        Self {
            window_dimensions: Dimensions {
                width: 420,
                height: 550,
            },
            window_padding: XY { x: 8, y: 8 },
            button_padding: XY { x: 8.0, y: 8.0 },
            button_with_preview_padding: XY { x: 5.0, y: 5.0 },
            button_corner_radius: 7,
            item_spacing: XY { x: 5.0, y: 5.0 },
            scroll_bar_margin: 8.0,
            pointer_gap: 5,
            screen_edge_gap: 10,
            preview_size: Dimensions {
                width: 105,
                height: 70,
            },
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct FontConfig {
    pub family: Option<String>,
    pub size: f32,
    pub secondary_size: f32,
    pub baseline_offset_factor: f32,
    pub underline_offset: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: None,
            size: 13.0,
            secondary_size: 11.0,
            baseline_offset_factor: 0.0,
            underline_offset: 0.0,
        }
    }
}

#[serde_as]
#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    #[serde_as(as = "DisplayFromStr")]
    pub background: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub foreground: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub muted_foreground: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub button_background: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub button_active_background: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub scroll_background: Color,
    #[serde_as(as = "DisplayFromStr")]
    pub scroll_handle: Color,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: Color(0xff191919),
            foreground: Color(0xffcccccc),
            muted_foreground: Color(0xff707070),
            button_background: Color(0xff2f2f2f),
            button_active_background: Color(0xff454545),
            scroll_background: Color(0xff0a0a0a),
            scroll_handle: Color(0xffbbbbbb),
        }
    }
}

impl Config {
    pub fn load() -> Result<Config> {
        let config_path = Self::get_config_path();

        if !config_path.exists() {
            return Ok(Config::default());
        }

        let config_content = fs::read_to_string(&config_path)?;
        let config: Config =
            toml::from_str(&config_content).context("Failed to parse config file")?;

        Ok(config)
    }

    fn get_config_path() -> PathBuf {
        let home_dir = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        let mut path = PathBuf::from(home_dir);
        path.push(".config");
        path.push("memoni");
        path.push("config.toml");
        path
    }
}

#[serde_as]
#[derive(Deserialize, Debug)]
pub struct Binding {
    #[serde_as(as = "FromInto<CharOrNum>")]
    pub key: u32,
    #[serde(default)]
    #[serde_as(as = "OneOrMany<_>")]
    pub modifiers: Vec<Modifier>,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    Control,
    Shift,
    Alt,
    Meta,
}

impl From<Modifier> for Keysym {
    fn from(value: Modifier) -> Self {
        match value {
            Modifier::Control => Keysym::Control_L,
            Modifier::Shift => Keysym::Shift_L,
            Modifier::Alt => Keysym::Alt_L,
            Modifier::Meta => Keysym::Meta_L,
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color(u32);

impl Deref for Color {
    type Target = u32;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl FromStr for Color {
    type Err = ParseColorError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Color32::from_hex(value)
            .map(|c| {
                Self(
                    ((c.a() as u32) << 24)
                        | ((c.r() as u32) << 16)
                        | ((c.g() as u32) << 8)
                        | (c.b() as u32),
                )
            })
            .map_err(ParseColorError)
    }
}

impl From<Color> for Color32 {
    fn from(value: Color) -> Self {
        let a = ((*value >> 24) & 0xff) as u8;
        let r = ((*value >> 16) & 0xff) as u8;
        let g = ((*value >> 8) & 0xff) as u8;
        let b = (*value & 0xff) as u8;
        Color32::from_rgba_unmultiplied(r, g, b, a)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParseColorError(ParseHexColorError);

impl Display for ParseColorError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match &self.0 {
            ParseHexColorError::MissingHash => write!(f, "invalid color: missing hash prefix"),
            ParseHexColorError::InvalidLength => {
                write!(f, "invalid color: invalid color string length")
            }
            ParseHexColorError::InvalidInt(int_err) => write!(f, "invalid color: {}", int_err),
        }
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct XY<T: Default> {
    pub x: T,
    pub y: T,
}

impl From<XY<f32>> for egui::Vec2 {
    fn from(val: XY<f32>) -> Self {
        egui::vec2(val.x, val.y)
    }
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u16,
    pub height: u16,
}

impl From<Dimensions> for egui::Vec2 {
    fn from(val: Dimensions) -> Self {
        egui::vec2(val.width.into(), val.height.into())
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum CharOrNum {
    Char(char),
    Num(u32),
}

impl From<CharOrNum> for u32 {
    fn from(value: CharOrNum) -> Self {
        match value {
            CharOrNum::Char(c) => c as u32,
            CharOrNum::Num(n) => n,
        }
    }
}
