//! Utilities for ROM management

use log::info;
use std::{fs, io, path::Path};

/// A GameBoy ROM
pub struct Rom {
    data: Vec<u8>,
}

impl Rom {
    pub fn load(path: &Path) -> io::Result<Self> {
        let data = fs::read(path)?;
        info!("Loaded ROM from {}", path.display());
        Ok(Self { data })
    }
}
