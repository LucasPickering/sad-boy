//! Game Boy clock emulation

use std::{
    cell::Cell,
    future,
    ops::{Add, AddAssign, Sub},
    task::Poll,
    time::Duration,
};
use tokio::time::{self, MissedTickBehavior};
use tracing::warn;

/// Number of dots (clock cycles) in a single frame
///
/// - https://gbdev.io/pandocs/Rendering.html
/// - https://josaphat.co/posts/gameboy-emulator/
const DOTS_PER_FRAME: Cycles = Cycles(70224);
/// Number of clock cycles per second (Hz)
///
/// The clock frequency is 2^22 Hz (~4.194 MHz).
const CLOCK_FREQUENCY: u32 = 1 << 22;
/// Elapsed time per clock cycle (dot)
const DOT_DURATION: Duration =
    Duration::from_secs(1).checked_div(CLOCK_FREQUENCY).unwrap();

/// Emulated hardware clock
///
/// The clock drives the CPU, GPU, and whatever other components run off the
/// main clock. TODO explain async stuff.
#[derive(Debug)]
pub struct Clock {
    /// Number of elapsed cycles (dots) **in the current frame**
    ///
    /// The max value for this is [DOTS_PER_FRAME] and resets to 0 at the
    /// beginning of every frame.
    cycles: Cell<u32>,
}

impl Clock {
    thread_local! {
        static CLOCK: Clock = Clock::new();
    }

    fn new() -> Self {
        Self {
            cycles: Cell::new(0),
        }
    }

    /// Get the number of cycles elapsed in the current frame
    pub fn elapsed() -> Cycles {
        Cycles(Self::CLOCK.with(|clock| clock.cycles.get()))
    }

    /// Run the CPU clock indefinitely
    ///
    /// TODO
    pub async fn run() {
        let mut interval = time::interval(DOT_DURATION);
        // If we can't keep up with the emulated speed, slow the whole thing
        // down. We don't want to skip ticks.
        //
        // Alternatively we could use Burst which will shorten the delays
        // between ticks, but that requires that there are actually delays.
        // When debugging performance issues, it's easier to have things run
        // slowly than not run at all.
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            Self::CLOCK.with(|clock| {
                // Increment the clock and wrap at the end of the frame
                let next = (clock.cycles.get() + 1) % DOTS_PER_FRAME.0;
                clock.cycles.set(next);
            });
        }
    }

    /// Wait for the given number of cycles to elapse
    ///
    /// This is how the CPU and GPU stay in sync. Each component waits some
    /// number of cycles, then at the end performs whatever work was meant to
    /// be done during those cycles. This simulates the time elapsed during a
    /// CPU instruction, GPU operation, etc.
    pub async fn wait(cycles: Cycles) {
        let current = Self::elapsed();
        let target = current + cycles;
        future::poll_fn(|_| {
            let current = Clock::elapsed();
            if current >= target {
                // Missing the exact match is a bug, could affect the
                // semantics
                if current > target {
                    warn!(?current, ?target, "Missed target clock cycle");
                }
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        })
        .await;
    }
}

/// Newtype for a number of clock cycles
///
/// This makes it clearer what a value is, instead of passing around `u32`
/// everywhere. Every executed instruction returns this value so the CPU can
/// report how many cycles were consumed from the budget.
#[derive(Clone, Copy, Debug, Default, Eq, Ord, PartialEq, PartialOrd)]
pub struct Cycles(pub u32);

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

impl From<u32> for Cycles {
    fn from(value: u32) -> Self {
        Self(value)
    }
}

impl Sub for Cycles {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}
