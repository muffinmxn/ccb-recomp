//! Game data loaded from the extracted WAD: configuration, strings and model archives.

use std::{collections::HashMap, path::{Path, PathBuf}};

use anyhow::{Context, Result};
use bevy::prelude::*;
use ccb_assets::{cfg::Config, msgs::Messages};
use wii_formats::lz;

#[derive(Resource)]
pub struct GameData {
    pub dir: PathBuf,
    pub cfg: Config,
    pub msgs: Messages,
    /// Decompressed BRRES archives by file name (without `.LZ`).
    brres: HashMap<String, Vec<u8>>,
}

impl GameData {
    pub fn load(dir: &Path) -> Result<Self> {
        Ok(Self {
            dir: dir.to_path_buf(),
            cfg: Config::load_dir(&dir.join("cfg"))?,
            msgs: Messages::load(&dir.join("text"), "en")?,
            brres: HashMap::new(),
        })
    }

    /// Reads a file from the data directory, decompressing `.LZ` files.
    pub fn read(&self, rel: &str) -> Result<Vec<u8>> {
        let raw = std::fs::read(self.dir.join(rel)).with_context(|| format!("reading {rel}"))?;
        Ok(if rel.ends_with(".LZ") && lz::is_lz(&raw) { lz::decompress(&raw)? } else { raw })
    }

    /// Loads (and caches) a model archive from `models/`.
    pub fn brres(&mut self, name: &str) -> Result<&[u8]> {
        let key = name.trim_end_matches(".LZ").to_string();
        if !self.brres.contains_key(&key) {
            let data = self.read(&format!("models/{name}"))?;
            self.brres.insert(key.clone(), data);
        }
        Ok(&self.brres[&key])
    }
}
