//! Emulation logic for a Nintendo Game Boy
//!
//! https://rylev.github.io/DMG-01/public/book/introduction.html

mod cpu;
mod gpu;
mod instruction;
mod memory;
mod rom;

use crate::{
    emu::{
        cpu::{Cpu, Cycles},
        gpu::Gpu,
        memory::{HIGH_RAM_LEN, Memory, MemoryBus, RAM_LEN},
        rom::Rom,
    },
    screen::Screen,
};
use color_eyre::eyre;
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
use tracing::error;

/// Number of dots (CPU cycles) in a single frame
///
/// - https://gbdev.io/pandocs/Rendering.html
/// - https://josaphat.co/posts/gameboy-emulator/
const DOTS_PER_FRAME: Cycles = Cycles(70224);
/// Real time in a single dot (CPU cycle)
///
/// The CPU frequency is 2^22 Hz (~4.194 MHz). The duration of one cycle is
/// the reciprocal of that.
///
/// Really this should be 238.4185791015625, but [Duration::from_secs_f64] isn't
/// `const`.
const DOT_DURATION: Duration = Duration::from_nanos(238);
/// Real time duration of a single frame
///
/// There are approximately 60 frames per second.
const FRAME_DURATION: Duration = Duration::from_nanos_u128(
    DOT_DURATION.as_nanos() * DOTS_PER_FRAME.0 as u128,
);

/// Game Boy emulator
#[derive(Debug)]
pub struct GameBoy {
    cpu: Cpu,
    gpu: Gpu,

    /// Read-only memory from the cartridge
    rom: Rom,
    /// General-purpose writable memory
    ///
    /// This is boxed because 8KiB is too big to reasonably put on the stack.
    ram: Memory<RAM_LEN>,
    /// Additional general-purpose writable memory
    ///
    /// This is most commonly used when accessed by the `LD HL, SP+imm8`
    /// instruction.
    high_ram: Memory<HIGH_RAM_LEN>,
}

impl GameBoy {
    /// Boot the Game Boy and load the ROM from a file
    pub fn boot(path: &Path) -> eyre::Result<Self> {
        let rom = Rom::load(path)?;
        Ok(Self {
            cpu: Cpu::default(),
            gpu: Gpu::default(),
            rom,
            ram: Memory::default(),
            high_ram: Memory::default(),
        })
    }

    /// Keep running until the CPU is halted
    pub fn run(&mut self, screen: &mut Screen) {
        // Each iteration of this loop is a single frame
        loop {
            screen.reset();
            let frame_start = Instant::now();
            let mut cycle_budget = DOTS_PER_FRAME;

            // Alternate between running the CPU and the PPU. The CPU runs a
            // single instruction whic htakes some number of cycles. Then we
            // run the PPU the same number of cycles to sync up.
            //
            // In reality these two components run concurrently, but a modern
            // CPU is so fast that we can flip-flop without any visible effect.
            //
            // The PPU needs to update after _every_ CPU instruction because the
            // PPU and CPU can affect each other:
            // - VRAM behavior based on PPU mode
            // - LCD registers can be modified mid-frame to change rendering
            while cycle_budget.0 > 0 {
                let mut memory = MemoryBus {
                    rom: &self.rom,
                    ram: &mut self.ram,
                    high_ram: &mut self.high_ram,
                    gpu: &mut self.gpu,
                };
                let cycles = self.cpu.execute_next(&mut memory);
                self.gpu.execute(cycles);
                cycle_budget.deduct(cycles);
            }
            if let Err(error) = screen.draw() {
                error!(%error, "Error drawing to screen");
            }

            // Sleep for the rest of the frame
            // It's possible this sleeps _too_ long, but the difference should
            // be negligible.
            // Unstable: use sleep_until
            // https://github.com/rust-lang/rust/issues/113752
            let elapsed = frame_start.elapsed();
            if let Some(sleep_time) = FRAME_DURATION.checked_sub(elapsed) {
                thread::sleep(sleep_time);
            }
        }
    }
}
