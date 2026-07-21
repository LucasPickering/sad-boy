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
use std::{
    path::Path,
    pin::pin,
    task::{Context, Poll, Waker},
};
use tracing::{Instrument, error, info_span};

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
        let clock = Clock::new();
        let memory_bus = MemoryBus {
            rom: &self.rom,
            ram: &mut self.ram,
            high_ram: &mut self.high_ram,
            gpu: &self.gpu,
        };
        let mut cpu_fut = pin!(
            self.cpu
                .run(&clock, memory_bus)
                .instrument(info_span!("CPU"))
        );
        let mut gpu_fut =
            pin!(self.gpu.run(&clock, screen).instrument(info_span!("GPU")));
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        loop {
            // These futures are supposed to be infinite loops, so if they exit
            // that's... odd
            let polls = [
                cpu_fut.as_mut().poll(&mut context),
                gpu_fut.as_mut().poll(&mut context),
            ];
            if polls.iter().any(Poll::is_ready) {
                error!("Future exited early");
                break;
            }
            clock.tick();
        }
    }
}
