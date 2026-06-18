//! Utilities for ROM management

use log::info;
use std::{fs, path::Path};

/// A GameBoy ROM
pub struct Rom {
    data: Vec<u8>,
}

impl Rom {
    pub fn load(path: &Path) -> Self {
        let data = fs::read(path).unwrap();
        info!("Loaded ROM from {}", path.display());
        Self { data }
    }
}
