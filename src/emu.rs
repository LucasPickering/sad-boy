//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod cpu;
mod instruction;
mod memory;
mod rom;

use crate::emu::{
    cpu::{Cpu, Cycles},
    memory::MemoryMap,
    rom::Rom,
};
use color_eyre::eyre;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// Game Boy emulator
#[derive(derive_more::Debug)]
pub struct GameBoy {
    cpu: Cpu,
    /// Virtual memory map
    #[debug(skip)]
    memory: MemoryMap,
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        let memory = MemoryMap::new(rom);
        Ok(Self {
            cpu: Cpu::default(),
            memory,
        })
    }

    /// Keep running until the CPU is halted
    pub fn run(&mut self) {
        /// https://josaphat.co/posts/gameboy-emulator/
        const CYCLES_PER_FRAME: Cycles = Cycles(70224);
        let frame_time = Duration::from_secs_f64(1.0 / 60.0);

        loop {
            let frame_start = Instant::now();
            self.cpu.run(&mut self.memory, CYCLES_PER_FRAME);

            // Sleep for the rest of the frame
            // It's possible this sleeps _too_ long, but the difference should
            // be negligible.
            // Unstable: use sleep_until
            // https://github.com/rust-lang/rust/issues/113752
            let elapsed = frame_start.elapsed();
            if let Some(sleep_time) = frame_time.checked_sub(elapsed) {
                thread::sleep(sleep_time);
            }
        }
    }
}
