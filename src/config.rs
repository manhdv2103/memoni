use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

#[derive(Deserialize, Debug)]
pub struct Config {
    pub terminals: HashSet<String>,
    pub style: StylingConfig,
}

#[derive(Deserialize, Debug)]
#[serde(default)]
pub struct StylingConfig {
    pub window_width: u16,
    pub window_height: u16,
    pub background_color: u32,
    pub font_size: f32,
    pub window_padding: i8,
    pub button_padding_x: f32,
    pub button_padding_y: f32,
    pub button_corner_radius: u8,
    pub item_spacing_x: f32,
    pub item_spacing_y: f32,
    pub scroll_bar_margin: f32,
    pub cursor_gap: i32,
}

impl Default for StylingConfig {
    fn default() -> Self {
        Self {
            window_width: 420,
            window_height: 550,
            background_color: 0x191919,
            font_size: 13.0,
            window_padding: 8,
            button_padding_x: 8.0,
            button_padding_y: 8.0,
            button_corner_radius: 7,
            item_spacing_x: 5.0,
            item_spacing_y: 5.0,
            scroll_bar_margin: 8.0,
            cursor_gap: 5,
        }
    }
}

impl Config {
    pub fn load() -> Result<Config> {
        let config_path = Self::get_config_path();

        if !config_path.exists() {
            return Ok(Config {
                terminals: HashSet::new(),
                style: StylingConfig::default(),
            });
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
