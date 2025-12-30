use anyhow::{Result, anyhow};
use log::{debug, info};
use std::{collections::VecDeque, fs, path::PathBuf};

use crate::{
    ordered_hash_map::OrderedHashMap,
    selection::{SelectionItem, SelectionMetadata, SelectionType},
};

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

    pub fn save_selection_data(
        &self,
        items: &OrderedHashMap<u64, SelectionItem>,
        metadata: &SelectionMetadata,
    ) -> Result<()> {
        info!("saving selection items to {:?}", self.file_path);
        let serialized_data = bincode::encode_to_vec((items, metadata), BINCODE_CONFIG)?;
        fs::write(&self.file_path, serialized_data)?;
        Ok(())
    }

    pub fn load_selection_data(
        &self,
    ) -> Result<(OrderedHashMap<u64, SelectionItem>, SelectionMetadata)> {
        if !self.file_path.exists() {
            info!("no persisted selection items file presented, skip loading");
            return Ok((OrderedHashMap::new(), SelectionMetadata::default()));
        }

        info!("loading selection items from {:?}", self.file_path);
        let data = fs::read(&self.file_path)?;

        let items: (OrderedHashMap<u64, SelectionItem>, SelectionMetadata) =
            match bincode::decode_from_slice(&data, BINCODE_CONFIG) {
                Ok((items, _)) => items,
                Err(err) => {
                    debug!("decoding failed, trying to decode using old format");
                    match bincode::decode_from_slice(&data, BINCODE_CONFIG) {
                        // TODO: remove support for old format
                        Ok((items, _)) => {
                            (convert_to_new_format(items), SelectionMetadata::default())
                        }
                        Err(old_format_err) => {
                            debug!("decoding using old format failed: {old_format_err}");
                            return Err(err.into());
                        }
                    }
                }
            };

        info!("{} items loaded", items.0.len());
        Ok(items)
    }
}

fn convert_to_new_format(old_items: VecDeque<SelectionItem>) -> OrderedHashMap<u64, SelectionItem> {
    let mut new_items = OrderedHashMap::new();
    for item in old_items {
        new_items.push_back(item.id, item);
    }
    new_items
}
