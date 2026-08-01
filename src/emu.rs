//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod clock;
mod cpu;
mod gpu;
mod instruction;
mod memory;
mod rom;

pub use clock::Clock;

use crate::{
    emu::{cpu::Cpu, gpu::Gpu, memory::MemoryBus, rom::Rom},
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
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        Ok(Self {
            cpu: Cpu::default(),
            gpu: Gpu::default(),
            rom,
        })
    }

    /// Initialize a Game Boy with a static ROM for testing
    #[cfg(test)]
    pub fn test(rom: Vec<u8>) -> Self {
        let rom = Rom::test(rom);
        Self {
            cpu: Cpu::default(),
            gpu: Gpu::default(),
            rom,
        }
    }

    /// Run the Game Boy indefinitely
    ///
    /// This will run until the given `stop_on` function returns `true`. It is
    /// called on every clock cycle.
    pub fn run(
        &mut self,
        screen: &mut dyn Screen,
        stop_on: impl Fn(&Clock) -> bool,
    ) {
        // The main loop uses futures to emulate components (CPU, GPU, etc.)
        // running concurrently. Async is used to make each component
        // incremental. Each component runs some discrete step then yields, and
        // the components are synced together by the emulated clock.
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

        // Run until the caller says to stop
        while !stop_on(&clock) {
            let polls = [
                cpu_fut.as_mut().poll(&mut context),
                gpu_fut.as_mut().poll(&mut context),
            ];
            // These futures are supposed to be infinite loops, so if they exit
            // that's... odd
            if polls.iter().any(Poll::is_ready) {
                error!("Future exited early");
                break;
            }
            clock.tick();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        emu::clock::Cycles,
        screen::{Color, HeadlessScreen},
        util::{TracingOutput, initialize_tracing},
    };
    use pretty_assertions::assert_eq;

    /// Run the emulator until the bootloader exits, then verify it matches the
    /// known state
    #[test]
    fn bootloader() {
        const BOOTLOADER_CYCLES: Cycles = Cycles(23_580_484);
        initialize_tracing(TracingOutput::Stderr);
        let rom_data = vec![0; memory::GAME_ROM.len()];
        let mut emu = GameBoy::test(rom_data);
        let mut screen =
            HeadlessScreen::new(SCREEN_WIDTH.into(), SCREEN_HEIGHT.into());
        // TODO figure out correct cycle length
        emu.run(&mut screen, |clock| clock.cycles() == BOOTLOADER_CYCLES);
        assert_eq!(emu.cpu, cpu::BOOTLOADER_EXPECTED);
        // TODO check memory/registers?
        // TODO look for logo
        screen.assert_pixels(&vec![
            Color::new(255, 255, 255);
            SCREEN_WIDTH as usize
                * SCREEN_HEIGHT as usize
        ]);
    }
}
