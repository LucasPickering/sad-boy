//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod clock;
mod cpu;
mod gpu;
mod instruction;
mod memory;
mod rom;

pub use clock::{Clock, Cycles};
pub use instruction::Instruction;

use crate::{
    backend::Backend,
    emu::{
        cpu::{Cpu, CpuDebugInfo},
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
    pin::Pin,
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
    ///
    /// Enabling the `debug` flag will start the emulator paused. It will also
    /// write additional information about the emulator state to the screen.
    pub fn run(&mut self, backend: &mut dyn Backend, debug: bool) {
        let mut frame =
            FrameBuffer::new(SCREEN_WIDTH.into(), SCREEN_HEIGHT.into());
        backend.draw(&frame); // Initialize the screen

        let waker = Waker::noop();
        let mut context = Context::from_waker(waker);
        // Macros are fun!
        macro_rules! poll {
            ($fut:expr) => {
                $fut.as_mut().is_none_or(|fut| {
                    fut.as_mut().poll(&mut context).is_ready()
                })
            };
        }

        // If the debugger is enabled, is execution paused?
        let mut debug_paused = true;
        // Read-only information about the current emulator state. This is
        // updated imperatively between instructions/frames so that it can be
        // read on any clock tick, regardless of the ownership state of various
        // futures
        let mut debug_info = DebugInfo::default();
        if debug {
            // Show initial debug state
            backend.debug(&debug_info);
        }

        // TODO explain all this shit
        // TODO remove boxing/dynamic shit
        let mut cpu_fut: Option<Pin<Box<dyn Future<Output = ()>>>> = None;
        let mut gpu_fut: Option<Pin<Box<dyn Future<Output = ()>>>> = None;

        loop {
            // Check for exit
            if backend.should_quit(&debug_info) {
                break;
            }

            // Progress the CPU
            if poll!(cpu_fut) {
                drop(cpu_fut);

                // Prep the next instruction
                let (fut, debug) = self.cpu.execute_next(
                    &self.clock,
                    MemoryBus::new(&mut self.memory, &self.rom, &self.gpu),
                );
                cpu_fut = Some(Box::pin(fut));
                debug_info.cpu = debug;
            }

            // Progress the GPU
            if poll!(gpu_fut) {
                drop(gpu_fut);

                // Draw the frame to the screen, then sleep until the end of the
                // frame so we stay synced up with the emulated clock speed
                backend.draw(&frame);
                frame.reset(); // Revert to all black
                self.clock.sleep();

                gpu_fut = Some(Box::pin(
                    self.gpu.render_frame(&self.clock, &mut frame),
                ));
            }

            // Check for input
            if debug && debug_paused {
                // If paused, we'll just block for input
                match backend.next_event_blocking() {
                    // Unpausing breaks out of this loop
                    InputEvent::DebugPauseToggle => debug_paused = false,
                    InputEvent::DebugStepNext => {} // Step one cycle
                    InputEvent::Quit => return,
                    // Any other input event while paused is ignored. Skip the
                    // rest of the loop and go back to waiting for input.
                    InputEvent::Button(_) => continue,
                }
            } else if self.clock.is_frame_start() {
                // On the first cycle of each frame, check for input
                while let Some(event) = backend.next_event() {
                    match event {
                        InputEvent::DebugPauseToggle => debug_paused ^= true,
                        InputEvent::DebugStepNext => {} // Does nothing
                        InputEvent::Quit => return,
                        InputEvent::Button(_) => {}
                    }
                }
            }

            self.clock.tick();
            // Update the debugger on each tick
            if debug {
                debug_info.clock_cycles = self.clock.cycles();
                backend.debug(&debug_info);
            }
        }
    }
}

/// Exposed debug state for the emulator
#[derive(Default)]
pub struct DebugInfo {
    pub clock_cycles: Cycles,
    pub cpu: CpuDebugInfo,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{backend::HeadlessBackend, emu::clock::Cycles, screen::Color};
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

    /// Run the emulator until the bootloader exits, then verify it matches the
    /// known state
    #[test]
    fn bootloader() {
        // Create an empty ROM that just holds the Nintendo logo. The bootloader
        // will load and display the logo from here
        let mut rom_data = vec![0; memory::CARTRIDGE_ROM_0.len()];
        rom_data[0x104..(0x104 + NINTENDO_LOGO.len())]
            .copy_from_slice(NINTENDO_LOGO);

        let mut emu = GameBoy::test(rom_data);
        // Run until the program counter hits the end of the bootloader. We
        // can't get safe access to the CPU registers because the CPU needs
        // mutable access to itself constantly. This raw pointer access is
        // pretty harmless.
        let mut backend = HeadlessBackend::new(move |debug_info| {
            debug_info.cpu.pc == memory::BOOTLOADER.last()
        });

        emu.run(&mut backend, false);

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
