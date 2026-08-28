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
pub use cpu::{BcdFlags, Cpu, InstructionInfo};
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
use std::{
    backtrace::Backtrace,
    fmt::{self, Display},
    path::Path,
    rc::Rc,
};
use tracing::error;

/// Game Boy emulator
#[derive(Clone, Debug)]
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
    /// [Fault] that occurred during a tick
    ///
    /// Once this is set, the emulator can no longer be progressed and the
    /// fault cannot be unset. If a fault occurs, the emulator may be in an
    /// unknown state so there is no way to recover.
    fault: Option<Fault>,
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
            fault: None,
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

    /// [Fault] that occurred during a tick
    ///
    /// Once this is set, the emulator can no longer be progressed and the
    /// fault cannot be unset. If a fault occurs, the emulator may be in an
    /// unknown state so there is no way to recover.
    pub fn fault(&self) -> Option<&Fault> {
        self.fault.as_ref()
    }

    /// Advance the emulator one clock cycle
    ///
    /// If this is the final clock cycle of the frame, this will sleep at the
    /// end of the tick to idle for the rest of the frame time.
    ///
    /// If a [Fault] occurs during the tick, it will be stored within the
    /// emulator and a reference to it is returned. Once a fault occurs, the
    /// emulator can no longer be progressed. Subsequent calls to [tick] will
    /// continue to return the same [Fault] without performing any work.
    pub fn tick(
        &mut self,
        backend: &mut dyn Backend,
    ) -> std::result::Result<(), &Fault> {
        // Shitty try block
        let mut tick_inner = || {
            // Tick *before* operations because it ensures the clock cycle
            // number seen by the emulator is the same as what's
            // visible externally after the tick (e.g. in the
            // debugger). This means tick #0 never really
            // happens, but that's okay. Zeroes are free.
            self.clock.tick();

            // Progress the CPU
            let mut memory_bus =
                MemoryBus::new(&self.rom, &mut self.ram, self.gpu.vram_mut());
            self.cpu.tick(&self.clock, &mut memory_bus)?;

            // Progress the GPU
            if self.gpu.tick(&self.clock, &mut self.frame)? {
                // Draw the frame to the screen
                backend.draw(&self.frame);
                self.frame.reset(); // Revert to all black
            }

            // Sleep at the end of the final tick of each frame to sync back
            // up with real time
            if self.clock.is_frame_end() {
                self.clock.sleep();
            }
            Ok(())
        };

        // If a fault is already set, we can't progress. Just return it
        if let Some(ref fault) = self.fault {
            return Err(fault);
        }

        // Run the tick. If it faults, store it
        tick_inner().map_err(|fault| {
            error!("FAULT: {fault:#}");
            self.fault.insert(fault) as &Fault
        })
    }
}

/// `Result<T, Fault>`
pub type FaultResult<T> = std::result::Result<T, Fault>;

/// An internal error in the emulator caused by a violated invariant
///
/// Faults are a recoverable form of a panic. They repesent invariant violations
/// and logical bugs. Any piece of emulator code that wants to assert on a
/// particular invariant will emit a fault if the assertion fails. This makes
/// it possible for the executor of the emulator to recover and display faults,
/// rather than crashing the entire process as a panic would.
///
/// Faults shouldn't be constructed manually; use [assert_fault] instead.
#[derive(Clone, Debug)]
pub struct Fault {
    /// Description of the fault
    message: String,
    /// Location where the fault occurred
    ///
    /// `Rc` is needed to make faulted emulators cloneable.
    backtrace: Rc<Backtrace>,
}

impl Fault {
    /// Create a new fault with a message and backtrace
    ///
    /// Caller has to capture the backtrace to ensure the trace is from the
    /// fault site.
    pub fn new(message: String, backtrace: Backtrace) -> Self {
        Self {
            message,
            backtrace: Rc::new(backtrace),
        }
    }
}

impl Display for Fault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)?;
        if f.alternate() {
            write!(f, "\n{}", self.backtrace)?;
        }
        Ok(())
    }
}

/// Assert that a condition is true, faulting if not
///
/// See [Fault] for a description of a fault. It's like panicking, but just for
/// the emulator.
macro_rules! assert_fault {
    ($expression:expr, $message:literal, $($arg:tt)*) => {
        if cfg!(debug_assertions) && !$expression {
            return Err($crate::emu::Fault::new(
                format!($message, $($arg)*),
                // Capture bt here so it points to the assertion
                std::backtrace::Backtrace::capture(),
            ));
        }
    };
}
pub(crate) use assert_fault;

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
            emulator.tick(&mut backend).unwrap();
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
