//! Game Boy clock emulation

use std::{
    cell::Cell,
    cmp::Ordering,
    future,
    ops::{Add, AddAssign, Sub},
    task::Poll,
    thread,
    time::{Duration, Instant},
};
use tracing::{debug, warn};

/// Number of dots (clock cycles) in a single frame
///
/// - https://gbdev.io/pandocs/Rendering.html
/// - https://josaphat.co/posts/gameboy-emulator/
const CYCLES_PER_FRAME: Cycles = Cycles(70224);
/// Number of clock cycles per second (Hz)
///
/// The clock frequency is 2^22 Hz (~4.194 MHz).
const CLOCK_FREQUENCY: u32 = 1 << 22;
/// Elapsed time per clock cycle (dot)
const CYCLE_DURATION: Duration =
    Duration::from_secs(1).checked_div(CLOCK_FREQUENCY).unwrap();

/// Emulated hardware clock
///
/// The clock drives the CPU, GPU, and whatever other components run off the
/// main clock. This uses `Cell`s so it can be handed out to each component's
/// future and still be ticked by the core emulator loop.
#[derive(Debug)]
pub struct Clock {
    /// Number of elapsed cycles (dots)
    ///
    /// This is monotonically increasing. For a `u64`, that gives us:
    /// ```
    /// 2^64 dots / 2^22 dots per second = 2^42 seconds
    /// ```
    /// That's a lot of years.
    cycles: Cell<Cycles>,
    /// TODO
    last_tick: Cell<Instant>,
    /// Number of cycles that missed the timing target **in the most recent
    /// frame**
    ///
    /// This is used just for logging.
    slow_cycles: Cell<u32>,
}

impl Clock {
    /// Initialize a new clock
    pub fn new() -> Self {
        Self {
            cycles: Cell::default(),
            last_tick: Instant::now().into(),
            slow_cycles: Cell::default(),
        }
    }

    /// Get the number of cycles completed in the current frame
    pub fn cycles(&self) -> Cycles {
        self.cycles.get()
    }

    /// Advance the clock one tick
    ///
    /// This will calculate how much time has elapsed since the last cycle was
    /// completed. It will sleep the thread the remaining duration of this clock
    /// cycle, then increment the cycle counter.
    pub fn tick(&self) {
        // How much of the cycle has already been consumed by real work?
        let elapsed = Instant::elapsed(&self.last_tick.get());
        // Sleep for the rest of the cycle
        if elapsed < CYCLE_DURATION {
            thread::sleep(elapsed);
        } else {
            // It's been longer than the cycle time since the last tick, which
            // means the future polling took longer than allowed. Unfortunately
            // we can't make time go backward (yet), so just log it and pray
            // we speed up.
            debug!("Slow cycle: {elapsed:?} > {CYCLE_DURATION:?}");
            self.slow_cycles.update(|v| v + 1);
        }

        // Increment the clock and wrap at the end of the frame
        self.cycles.update(|cycles| cycles + Cycles(1));
        if self.cycles.get().0.is_multiple_of(CYCLES_PER_FRAME.0) {
            // Every frame, check the number of slow cycles
            let slow = self.slow_cycles.replace(0);
            if slow > 0 {
                warn!(
                    "{slow}/{total} cycles in this frame were slow",
                    total = CYCLES_PER_FRAME.0
                );
            }
        }
        self.last_tick.set(Instant::now());
    }

    /// Wait for the given number of cycles to elapse
    ///
    /// This is how the CPU and GPU stay in sync. Each component waits some
    /// number of cycles, then at the end performs whatever work was meant to
    /// be done during those cycles. This simulates the time elapsed during a
    /// CPU instruction, GPU operation, etc.
    pub async fn wait(&self, cycles: Cycles) {
        let current = self.cycles.get();
        let target = current + cycles;
        future::poll_fn(|_| {
            let current = self.cycles.get();
            match current.cmp(&target) {
                Ordering::Less => Poll::Pending,
                Ordering::Equal => Poll::Ready(()),
                Ordering::Greater => {
                    // This *should* be impossible because every future gets
                    // polled on every clock cycle. Missing
                    // cycles could affect semantics
                    warn!(?current, ?target, "Missed target clock cycle");
                    Poll::Ready(())
                }
            }
        })
        .await;
    }
}

/// Newtype for a number of clock cycles
///
/// This makes it clearer what a value is, instead of passing around a bare
/// integer everywhere. Every executed instruction returns this value so the CPU
/// can report how many cycles were consumed from the budget.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Cycles(pub u64);

impl Add for Cycles {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl AddAssign for Cycles {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl From<u64> for Cycles {
    fn from(value: u64) -> Self {
        Self(value)
    }
}

impl Sub for Cycles {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}
