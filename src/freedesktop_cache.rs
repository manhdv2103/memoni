use std::{
    os::unix::ffi::OsStrExt as _,
    path::{self, Path, PathBuf},
};

use anyhow::{Result, anyhow};
use md5::{Digest, Md5};

use crate::utils::{percent_encode, to_hex_string};

pub fn get_cached_thumbnail<P: AsRef<Path>>(file: P) -> Result<Option<PathBuf>> {
    let thumbnails_dir = dirs::cache_dir()
        .ok_or_else(|| anyhow!("cache directory not found"))?
        .join("thumbnails");

    let is_cached_thumbnail = file.as_ref().ancestors().any(|a| a == thumbnails_dir);
    if is_cached_thumbnail {
        return Ok(Some(file.as_ref().to_path_buf()));
    }

    let mut hasher = Md5::new();
    hasher.update(b"file://");
    for component in path::absolute(file)?.components().skip(1) {
        hasher.update(b"/");
        hasher.update(percent_encode(component.as_os_str().as_bytes()));
    }

    let thumbnail_name = to_hex_string(&hasher.finalize());
    let thumbnail_filename = format!("{}.png", thumbnail_name);

    for size in &["normal", "large", "x-large", "xx-large"] {
        let thumbnail = thumbnails_dir.join(size).join(&thumbnail_filename);
        if thumbnail.exists() && thumbnail.is_file() {
            return Ok(Some(thumbnail));
        }
    }

    Ok(None)
}
