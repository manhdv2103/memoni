use anyhow::{Result, anyhow};
use std::{collections::VecDeque, fs, path::PathBuf};

use crate::selection::{SelectionItem, SelectionType};

const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();

pub struct Persistence {
    file_path: PathBuf,
}

impl Persistence {
    pub fn new(selection_type: SelectionType) -> Result<Self> {
        let xdg_data_home = dirs::data_dir()
            .ok_or_else(|| anyhow!("data directory not found"))?
            .join("memoni");
        fs::create_dir_all(&xdg_data_home)?;

        let file_name = format!("{}_selections", selection_type.to_string().to_lowercase());
        let file_path = xdg_data_home.join(file_name);

        Ok(Persistence { file_path })
    }

    pub fn save_selection_items(&self, items: &VecDeque<SelectionItem>) -> Result<()> {
        let serialized_data = bincode::encode_to_vec(items, BINCODE_CONFIG)?;
        fs::write(&self.file_path, serialized_data)?;
        Ok(())
    }

    pub fn load_selection_items(&self) -> Result<VecDeque<SelectionItem>> {
        if !self.file_path.exists() {
            return Ok(VecDeque::new());
        }

        let data = fs::read(&self.file_path)?;
        let (items, _): (VecDeque<SelectionItem>, usize) =
            bincode::decode_from_slice(&data, BINCODE_CONFIG)?;
        Ok(items)
    }
}
