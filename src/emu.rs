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
    emu::{clock::Clock, cpu::Cpu, gpu::Gpu, memory::MemoryBus, rom::Rom},
    screen::Screen,
};
use color_eyre::eyre;
use std::{
    path::Path,
    pin::pin,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
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
    /// Flag that will be set when the emulator should exit
    quit: Arc<AtomicBool>,
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path, quit: Arc<AtomicBool>) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        Ok(Self {
            cpu: Cpu::default(),
            gpu: Gpu::default(),
            rom,
            quit,
        })
    }

    /// Run the Game Boy indefinitely
    ///
    /// This will never return. To stop the Game Boy, kill the process.
    pub fn run(self, screen: &mut Screen) {
        // TODO explain main loop
        let clock = Clock::new();
        let memory_bus = MemoryBus::new(&self.rom, &self.gpu);
        let mut cpu_fut = pin!(
            self.cpu
                .run(&clock, memory_bus)
                .instrument(info_span!("CPU"))
        );
        let mut gpu_fut =
            pin!(self.gpu.run(&clock, screen).instrument(info_span!("GPU")));
        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        while !self.quit.load(Ordering::Relaxed) {
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
