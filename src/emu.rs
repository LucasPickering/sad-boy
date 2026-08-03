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
    backend::Backend,
    emu::{
        cpu::Cpu,
        gpu::Gpu,
        memory::{Memory, MemoryBus},
        rom::Rom,
    },
    input::InputEvent,
    screen::FrameBuffer,
};
use color_eyre::eyre;
use std::{
    path::Path,
    pin::pin,
    task::{Context, Waker},
};

/// Width of the screen in pixels
const SCREEN_WIDTH: u8 = 160;
/// Height of the screen in pixels
const SCREEN_HEIGHT: u8 = 144;

/// Game Boy emulator
#[derive(Debug)]
pub struct GameBoy {
    clock: Clock,
    cpu: Cpu,
    gpu: Gpu,
    /// Read-only memory from the cartridge
    rom: Rom,
    memory: Memory,
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
            cpu: Cpu::default(),
            gpu: Gpu::default(),
            rom,
            memory: Memory::default(),
        }
    }

    /// Run the Game Boy indefinitely
    ///
    /// This will run until the given `stop_on` function returns `true`. It is
    /// called on every clock cycle.
    pub fn run(&mut self, backend: &mut dyn Backend) {
        let mut frame =
            FrameBuffer::new(SCREEN_WIDTH.into(), SCREEN_HEIGHT.into());
        backend.draw(&frame); // Initialize the screen

        // The main loop uses futures to emulate components (CPU, GPU, etc.)
        // running concurrently. Async is used to make each component
        // incremental. Each component runs some discrete step then yields, and
        // the components are synced together by the emulated clock.
        let memory_bus = MemoryBus::new(&mut self.memory, &self.rom, &self.gpu);
        let mut cpu_fut = pin!(self.cpu.run(&self.clock, memory_bus));

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        macro_rules! poll {
            ($fut:expr) => {
                $fut.as_mut().poll(&mut context)
            };
        }

        // Run until the caller says to stop. Each iteration of this outer loop
        // is one frame
        while !backend.should_quit() {
            // Run ticks until the end of the frame. This will tick as quickly
            // as possible, limited just by processing speed.
            //
            // Each frame uses a different future in the GPU so we can read back
            // from the frame buffer after writing to it.
            {
                let mut gpu_fut = pin!(self.gpu.frame(&self.clock, &mut frame));
                while poll!(gpu_fut).is_pending() {
                    // Run the CPU on each tick too
                    let cpu_poll = poll!(cpu_fut);
                    // The CPU future is supposed to run indefinitely
                    assert!(cpu_poll.is_pending(), "CPU future exited early");
                    // Advance the clock one tick
                    self.clock.tick();
                }
            }

            // Draw the frame to the screen, then sleep until the end of the
            // frame so we stay synced up with the emulated clock speed
            backend.draw(&frame);

            // Check for input
            while let Some(event) = backend.next_event() {
                match event {
                    InputEvent::Quit => return, // We done
                    InputEvent::StepNext => {}
                }
            }

            // TODO reset the frame buffer?
            self.clock.sleep();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        backend::HeadlessBackend,
        emu::{clock::Cycles, memory::Address},
        screen::Color,
        util::{TracingOutput, initialize_tracing},
    };
    use pretty_assertions::assert_eq;
    use std::ptr;

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

    /// Run the emulator until the bootloader exits, then verify it matches the
    /// known state
    #[test]
    fn bootloader() {
        initialize_tracing(TracingOutput::Stderr); // TODO remove

        // Create an empty ROM that just holds the Nintendo logo. The bootloader
        // will load and display the logo from here
        let mut rom_data = vec![0; memory::GAME_ROM.len()];
        rom_data[0x104..(0x104 + NINTENDO_LOGO.len())]
            .copy_from_slice(NINTENDO_LOGO);

        let mut emu = GameBoy::test(rom_data);
        // Run until the program counter hits the end of the bootloader. We
        // can't get safe access to the CPU registers because the CPU needs
        // mutable access to itself constantly. This raw pointer access is
        // pretty harmless.
        let pc_ptr = ptr::from_ref::<Address>(emu.cpu.pc());
        let mut backend = HeadlessBackend::new(
            // SAFETY: The emulator and CPU are never moved in memory
            move || unsafe { *pc_ptr } == memory::BOOTLOADER.last(),
        );

        emu.run(&mut backend);

        assert_eq!(emu.cpu, cpu::BOOTLOADER_EXPECTED);
        assert_eq!(emu.memory.bank(), 1);
        // TODO look for logo
        backend.assert_pixels(&vec![
            Color::new(255, 255, 255);
            SCREEN_WIDTH as usize
                * SCREEN_HEIGHT as usize
        ]);
        assert_eq!(emu.clock.cycles(), Cycles(23_580_484));
    }
}
