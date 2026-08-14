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
pub use cpu::{CpuDebugInfo, InstructionDebugInfo};
pub use instruction::Instruction;
pub use memory::Address;

use crate::{
    Executor,
    backend::FrameBuffer,
    emu::{
        cpu::Cpu,
        gpu::Gpu,
        memory::{Memory, MemoryBus},
        rom::Rom,
    },
};
use color_eyre::eyre;
use std::{
    path::Path,
    task::{Context, Waker},
};

/// Width of the screen in pixels
const SCREEN_WIDTH: u8 = 160;
/// Height of the screen in pixels
const SCREEN_HEIGHT: u8 = 144;

/// Poll a future and if ready, drop it and initialize a new one
///
/// This is built specifically for the main loop, which uses `Option` futures
/// that get reinitialized periodically. The passed `init` expression can also
/// do any computations it needs to between futures. This is needed for some
/// expressions that need access to references that would normally be
/// exclusively given to the future.
///
/// This has to be implemented as a macro (rather than a struct) because the
/// futures rely heavily on local scope to manage borrows. The emulator needs
/// certain accesses between futures that are only possible in a local namespace
/// where drops and reassignments are more flexible.
macro_rules! poll {
    ($fut:ident, $ctx:expr, $init:expr) => {
        let is_ready = $fut.as_mut().poll(&mut $ctx).is_ready();
        if is_ready {
            // Drop the previous future to release all its references. Then
            // evaluate $init, which will do whatever between-futures logic it
            // needs before returning a new future.
            drop($fut);
            let mut fut = Box::pin($init);
            // Poll the new future once to let it initialize itself.
            // Particularly, this ensures it hits its first clock.wait() and
            // that the target clock cycle in the wait is correct. If we delay
            // this first poll to the next loop iteration, it will lose a clock
            // cycle.
            //
            // The poll output doesn't matter here, we're just going to poll
            // again.
            let _ = fut.as_mut().poll(&mut $ctx);
            $fut = fut;
        }
    };
}

/// Game Boy emulator
#[derive(Debug)]
pub struct GameBoy {
    clock: Clock,
    cpu: Cpu,
    gpu: Gpu,
    /// Read-only memory from the cartridge
    rom: Rom,
    memory: Memory,
    // Debug state
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
    pub fn run(&mut self, stepper: &mut Executor) {
        // Set debug info to the initial system state
        stepper.update_debug_info(|info| {
            let mut memory_bus =
                MemoryBus::new(&mut self.memory, &self.rom, &self.gpu);
            info.cpu = self.cpu.debug_info(&self.clock, &mut memory_bus);
        });

        let mut frame =
            FrameBuffer::new(SCREEN_WIDTH.into(), SCREEN_HEIGHT.into());
        stepper.draw(&frame); // Initialize the screen

        // TODO explain all this shit
        let mut context = Context::from_waker(Waker::noop());
        // These futures have to be boxed so they can be pinned. Theoretically
        // I think stack pinning should be possible, but it's hard because of
        // the scopes. The binding gets dropped and reassigned from within the
        // loop. It can't stack-pin it from inside the inner loop. If the boxing
        // turns out to be a perf problem I'll reevaluate.
        let mut cpu_fut = Box::pin(self.cpu.execute_next(
            &self.clock,
            MemoryBus::new(&mut self.memory, &self.rom, &self.gpu),
        ));
        let mut gpu_fut =
            Box::pin(self.gpu.render_frame(&self.clock, &mut frame));

        // Each iteration of this loop is a single clock cycle
        loop {
            // The stepper controls iteration; on each loop, check if/when we
            // should continue. When the debugger is paused, this will block
            // until it's time to continue.
            if stepper.next().is_break() {
                break;
            }

            // Progress the CPU
            poll!(cpu_fut, context, {
                let mut memory_bus =
                    MemoryBus::new(&mut self.memory, &self.rom, &self.gpu);
                // Update debug info between instructions
                stepper.update_debug_info(|info| {
                    info.cpu =
                        self.cpu.debug_info(&self.clock, &mut memory_bus);
                });

                self.cpu.execute_next(&self.clock, memory_bus)
            });

            // Progress the GPU
            poll!(gpu_fut, context, {
                // Draw the frame to the screen
                stepper.draw(&frame);
                frame.reset(); // Revert to all black

                self.gpu.render_frame(&self.clock, &mut frame)
            });

            // Update the debugger on each tick
            stepper.update_debug_info(|info| {
                info.clock_cycles = self.clock.cycles();
            });

            // Sleep at the end of the final tick of each frame to sync back
            // up with real time
            if self.clock.is_frame_end() {
                self.clock.sleep();
            }
            self.clock.tick();
        }
    }
}

/// Exposed read-only state for the emulator
#[derive(Default)]
pub struct DebugInfo {
    /// Number of elapsed clock cycles since boot
    pub clock_cycles: Cycles,
    /// CPU state
    pub cpu: CpuDebugInfo,
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
        let mut stepper = Executor::new(&mut backend);

        emu.run(&mut stepper);

        assert_eq!(emu.cpu, cpu::BOOTLOADER_EXPECTED);
        assert_eq!(emu.memory.bank(), 1);
        // TODO look for logo
        let expected = FrameBuffer::test(
            SCREEN_WIDTH.into(),
            SCREEN_HEIGHT.into(),
            vec![
                Color::new(255, 255, 255);
                SCREEN_WIDTH as usize * SCREEN_HEIGHT as usize
            ],
        );
        backend.assert_pixels(&expected);
        assert_eq!(emu.clock.cycles(), Cycles(23_580_484));
    }
}
