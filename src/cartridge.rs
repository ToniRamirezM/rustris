use std::fs::File;
use std::io::{Read, Result};

pub struct Cartridge {
    pub rom: Vec<u8>,
}

// Cartridge emulation: reads the entire ROM file into memory as a byte vector.
impl Cartridge {
    pub fn from_file(path: &str) -> Result<Self> {
        let mut file = File::open(path)?;
        let mut rom = Vec::new();
        file.read_to_end(&mut rom)?;
        Ok(Cartridge { rom })
    }
}
