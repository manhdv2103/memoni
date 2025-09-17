use anyhow::{Context, Result};
use serde::Deserialize;
use serde_with::{FromInto, OneOrMany, serde_as};
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
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
    pub button_corner_radius: u8,
    pub item_spacing: XY<f32>,
    pub scroll_bar_margin: f32,
    pub pointer_gap: i32,
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
            button_corner_radius: 7,
            item_spacing: XY { x: 5.0, y: 5.0 },
            scroll_bar_margin: 8.0,
            pointer_gap: 5,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct FontConfig {
    pub family: Option<String>,
    pub size: f32,
    pub baseline_offset_factor: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: None,
            size: 13.0,
            baseline_offset_factor: 0.0,
        }
    }
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct ThemeConfig {
    pub background: u32,
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: 0x191919,
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
pub struct XY<T: Default> {
    pub x: T,
    pub y: T,
}

#[derive(Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u16,
    pub height: u16,
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
