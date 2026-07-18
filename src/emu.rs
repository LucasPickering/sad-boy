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
        memory::{Memory, MemoryBus},
        rom::Rom,
    },
    screen::Screen,
};
use color_eyre::eyre;
use std::{cell::Cell, path::Path, time::Duration};
use tokio::{runtime::LocalRuntime, sync::broadcast, time};

/// Width of the screen in pixels
pub const SCREEN_WIDTH: u16 = 160;
/// Height of the screen in pixels
pub const SCREEN_HEIGHT: u16 = 144;

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
        };
        runtime.block_on(async move {
            futures::join!(
                Clock::run(),
                self.cpu.run(memory),
                self.gpu.run(screen),
            );
        });
    }
}

/// TODO
#[derive(Debug)]
struct Clock {
    /// TODO
    cycles: Cell<u32>,
    broadcast: broadcast::Sender<u32>,
}

impl Clock {
    thread_local! {
        static CLOCK: Clock = Clock::new();
    }

    fn new() -> Self {
        let (broadcast, _) = broadcast::channel(16);
        Self {
            cycles: Cell::new(0),
            broadcast,
        }
    }

    /// Get the number of cycles elapsed in the current frame
    pub fn elapsed() -> Cycles {
        Cycles(Self::CLOCK.with(|clock| clock.cycles.get()))
    }

    /// Run the CPU clock indefinitely
    ///
    /// TODO
    async fn run() {
        let mut interval = time::interval(DOT_DURATION);
        loop {
            interval.tick().await;
            Self::CLOCK.with(|clock| {
                // Increment the clock and wrap at the end of the frame
                let next = (clock.cycles.get() + 1) % DOTS_PER_FRAME.0;
                clock.cycles.set(next);
                // Send failure just means there's no receivers RIGHT NOW. More
                // could come later.
                let _ = clock.broadcast.send(next);
            });
        }
    }

    /// Wait for the given number of cycles to elapse
    ///
    /// This is how the CPU and GPU stay in sync. Each component waits some
    /// number of cycles, then at the end performs whatever work was meant to
    /// be done during those cycles. This simulates the time elapsed during a
    /// CPU instruction, GPU operation, etc.
    async fn wait(cycles: Cycles) {
        let (current, mut rx) = Self::CLOCK
            .with(|clock| (clock.cycles.get(), clock.broadcast.subscribe()));
        let target = current + cycles.0;
        // The broadcast sends a message after each cycle
        while let Ok(current) = rx.recv().await {
            if current >= target {
                break;
            }
        }
    }
}
