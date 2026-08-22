//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod clock;
mod cpu;
mod gpu;
pub mod instruction;
pub mod memory;
mod rom;

pub use clock::{Clock, Cycles};
pub use cpu::{Cpu, InstructionInfo};
pub use memory::{Address, AddressRange, MemoryBusReadOnly};

use crate::{
    backend::{Backend, FrameBuffer},
    emu::{
        gpu::Gpu,
        memory::{MemoryBus, RandomAccessMemory},
        rom::Rom,
    },
};
use color_eyre::eyre;
use std::path::Path;

/// Game Boy emulator
#[derive(Debug)]
pub struct GameBoy {
    clock: Clock,
    cpu: Cpu,
    gpu: Gpu,
    /// Read-only memory from the cartridge
    rom: Rom,
    ram: RandomAccessMemory,
    /// Next frame to be drawn to the screen
    ///
    /// This is incrementally updated by the GPU during a frame, then pushed
    /// to the screen all at once.
    frame: FrameBuffer,
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        Ok(Self::new(rom))
    }

    /// Initialize a Game Boy with a static ROM for testing
    #[cfg(test)]
    pub fn test(rom: Vec<u8>) -> Self {
        Self::new(Rom::test(rom))
    }

    fn new(rom: Rom) -> Self {
        Self {
            clock: Clock::new(),
            cpu: Cpu::new(),
            gpu: Gpu::default(),
            rom,
            ram: RandomAccessMemory::default(),
            frame: FrameBuffer::new(),
        }
    }

    /// Get the system clock
    pub fn clock(&self) -> &Clock {
        &self.clock
    }

    /// Get the CPU state
    pub fn cpu(&self) -> &Cpu {
        &self.cpu
    }

    /// Get a read-only memory view
    pub fn memory(&self) -> MemoryBusReadOnly<'_> {
        MemoryBusReadOnly {
            ram: &self.ram,
            rom: &self.rom,
            vram: self.gpu.vram(),
        }
    }

    /// Get the in-memory frame buffer
    pub fn frame(&self) -> &FrameBuffer {
        &self.frame
    }

    /// Advance the emulator one clock cycle
    ///
    /// If this is the final clock cycle of the frame, this will sleep at the
    /// end of the tick to idle for the rest of the frame time.
    pub fn tick(&mut self, backend: &mut dyn Backend) {
        // Tick *before* operations because it ensures the clock cycle number
        // seen by the emulator is the same as what's visible externally after
        // the tick (e.g. in the debugger). This means tick #0 never really
        // happens, but that's okay. Zeroes are free.
        self.clock.tick();

        // Progress the CPU
        let mut memory_bus =
            MemoryBus::new(&mut self.ram, &self.rom, self.gpu.vram_mut());
        self.cpu.tick(&self.clock, &mut memory_bus);

        // Progress the GPU
        if self.gpu.tick(&self.clock, &mut self.frame) {
            // Draw the frame to the screen
            backend.draw(&self.frame);
            self.frame.reset(); // Revert to all black
        }

        // Sleep at the end of the final tick of each frame to sync back
        // up with real time
        if self.clock.is_frame_end() {
            self.clock.sleep();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::{Color, HeadlessBackend},
        emu::clock::Cycles,
    };
    use pretty_assertions::assert_eq;

    /// Encoded Nintendo logo and TM symbol
    ///
    /// https://gbdev.gg8.se/wiki/articles/Gameboy_Bootstrap_ROM#The_DMG_bootstrap
    const NINTENDO_LOGO: &[u8] = &[
        0xCE, 0xED, 0x66, 0x66, 0xCC, 0x0D, 0x00, 0x0B, 0x03, 0x73, 0x00, 0x83,
        0x00, 0x0C, 0x00, 0x0D, 0x00, 0x08, 0x11, 0x1F, 0x88, 0x89, 0x00, 0x0E,
        0xDC, 0xCC, 0x6E, 0xE6, 0xDD, 0xDD, 0xD9, 0x99, 0xBB, 0xBB, 0x67, 0x63,
        0x6E, 0x0E, 0xEC, 0xCC, 0xDD, 0xDC, 0x99, 0x9F, 0xBB, 0xB9, 0x33, 0x3E,
        0x3C, 0x42, 0xB9, 0xA5, 0xB9, 0xA5, 0x42, 0x3C,
    ];

    /// Run the emulator until the bootstrap exits, then verify it matches the
    /// known state
    #[test]
    fn bootstrap() {
        // Create an empty ROM that just holds the Nintendo logo. The bootstrap
        // will load and display the logo from here
        let mut rom_data = vec![0; memory::CARTRIDGE_ROM_0.len()];
        rom_data[0x104..(0x104 + NINTENDO_LOGO.len())]
            .copy_from_slice(NINTENDO_LOGO);

        let mut backend = HeadlessBackend::new();
        let mut emulator = GameBoy::test(rom_data);
        // Run until the program counter hits the end of the bootstrap
        while emulator.cpu.registers().pc() <= memory::BOOTSTRAP.last() {
            emulator.tick(&mut backend);
        }

        assert_eq!(emulator.ram.bank(), 1);
        assert_eq!(emulator.clock.cycles(), Cycles(23_580_484));
        // TODO check CPU state

        // TODO look for logo
        let expected = FrameBuffer::from_pixels(vec![
            Color::new(255, 255, 255);
            FrameBuffer::LENGTH
        ]);
        backend.assert_pixels(&expected);
    }
}
