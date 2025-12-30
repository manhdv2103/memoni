use anyhow::{Result, anyhow};
use log::{debug, info};
use std::{
    collections::VecDeque,
    fs::{self, File},
    io::{Read, Write as _},
    path::PathBuf,
};

use crate::{
    ordered_hash_map::OrderedHashMap,
    selection::{SelectionItem, SelectionMetadata, SelectionType},
};

const BINCODE_CONFIG: bincode::config::Configuration = bincode::config::standard();
const BINARY_VERSION: u32 = 2;

pub struct Persistence {
    file_path: PathBuf,
    temp_file_path: PathBuf,
}

impl Persistence {
    pub fn new(selection_type: SelectionType) -> Result<Self> {
        let xdg_data_home = dirs::data_dir()
            .ok_or_else(|| anyhow!("data directory not found"))?
            .join("memoni");
        fs::create_dir_all(&xdg_data_home)?;

        let file_name = format!("{}_selections", selection_type.to_string().to_lowercase());
        let file_path = xdg_data_home.join(file_name);
        let temp_file_path = file_path.with_extension("tmp");

        Ok(Persistence {
            file_path,
            temp_file_path,
        })
    }

    pub fn save_selection_data(
        &self,
        items: &OrderedHashMap<u64, SelectionItem>,
        metadata: &SelectionMetadata,
    ) -> Result<()> {
        info!("saving selection items to {:?}", self.file_path);
        let serialized_data = bincode::encode_to_vec((items, metadata), BINCODE_CONFIG)?;

        let mut f = File::create(&self.temp_file_path)?;
        f.write_all(&BINARY_VERSION.to_le_bytes())?;
        f.write_all(&serialized_data)?;
        f.sync_all()?;
        fs::rename(&self.temp_file_path, &self.file_path)?;

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
        let mut file = File::open(&self.file_path)?;

        let mut version_buf = [0u8; 4];
        file.read_exact(&mut version_buf)?;
        let version = u32::from_le_bytes(version_buf);

        let mut data = Vec::new();
        file.read_to_end(&mut data)?;

        let items: Result<(OrderedHashMap<u64, SelectionItem>, SelectionMetadata)> = match version {
            // version 1 does not have version field unfortunately
            2 => bincode::decode_from_slice(&data, BINCODE_CONFIG)
                .map(|(items, _)| items)
                .map_err(Into::into),
            _ => Err(anyhow!("invalid binary version")),
        };

        let items = (match items {
            Ok(items) => Ok(items),
            Err(err) => {
                debug!("decoding failed, trying to decode using version 1 format");
                data.splice(0..0, version_buf);
                decode_version_1(&data).map_err(|ver1_err| {
                    debug!("decoding using version 1 format failed: {ver1_err}");
                    err
                })
            }
        })?;

        info!("{} items loaded", items.0.len());
        Ok(items)
    }
}

fn decode_version_1(
    data: &[u8],
) -> Result<(OrderedHashMap<u64, SelectionItem>, SelectionMetadata)> {
    let old_items: VecDeque<SelectionItem> = bincode::decode_from_slice(data, BINCODE_CONFIG)?.0;
    let mut new_items = OrderedHashMap::new();
    for item in old_items {
        new_items.push_back(item.id, item);
    }

    Ok((new_items, SelectionMetadata::default()))
}
