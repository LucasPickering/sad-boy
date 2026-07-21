//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod clock;
mod cpu;
mod gpu;
mod instruction;
mod memory;
mod rom;

use crate::{
    emu::{
        clock::Clock,
        cpu::Cpu,
        gpu::Gpu,
        memory::{Memory, MemoryBus},
        rom::Rom,
    },
    screen::Screen,
};
use color_eyre::eyre;
use std::path::Path;
use tokio::runtime::LocalRuntime;
use tracing::{Instrument, info_span};

/// Width of the screen in pixels
pub const SCREEN_WIDTH: u8 = 160;
/// Height of the screen in pixels
pub const SCREEN_HEIGHT: u8 = 144;

/// Game Boy emulator
#[derive(Debug)]
pub struct GameBoy {
    cpu: Cpu,
    gpu: Gpu,

    /// Read-only memory from the cartridge
    rom: Rom,
    /// General-purpose writable memory
    ram: Memory<u8>,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used via the `LD HL, SP+imm8` instruction.
    high_ram: Memory<u8>,
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        Ok(Self {
            cpu: Cpu::default(),
            gpu: Gpu::default(),
            rom,
            ram: Memory::new(memory::RAM),
            high_ram: Memory::new(memory::HIGH_RAM),
        })
    }

    /// Run the Game Boy indefinitely
    ///
    /// This will never return. To stop the Game Boy, kill the process.
    pub fn run(mut self, screen: &mut Screen) {
        // TODO explain
        let runtime = LocalRuntime::new().unwrap();
        let memory = MemoryBus {
            rom: &self.rom,
            ram: &mut self.ram,
            high_ram: &mut self.high_ram,
            gpu: &self.gpu,
        };
        runtime.block_on(async {
            futures::join!(
                Clock::run().instrument(info_span!("Clock")),
                self.cpu.run(memory).instrument(info_span!("CPU")),
                self.gpu.run(screen).instrument(info_span!("GPU")),
            );
        });
    }
}
