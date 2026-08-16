//! Game Boy clock emulation

use std::{
    fmt::{self, Display},
    ops::{Add, AddAssign, Mul, Sub},
    thread,
    time::{Duration, Instant},
};
use tracing::warn;

/// Emulated hardware clock
///
/// The clock drives the CPU, GPU, and whatever other components run off the
/// main clock.
#[derive(Debug)]
pub struct Clock {
    /// Number of elapsed cycles (dots)
    ///
    /// This is monotonically increasing. For a `u64`, that gives us:
    /// ```
    /// 2^64 dots / 2^22 dots per second = 2^42 seconds
    /// ```
    /// That's a lot of years.
    cycles: Cycles,
    /// Moment when the final tick of the previous frame was completed
    frame_start: Instant,
}

impl Clock {
    /// Number of dots (clock cycles) in a single frame
    ///
    /// - https://gbdev.io/pandocs/Rendering.html
    /// - https://josaphat.co/posts/gameboy-emulator/
    pub const CYCLES_PER_FRAME: Cycles = Cycles(70224);

    /// Number of clock cycles per second (Hz)
    ///
    /// The clock frequency is 2^22 Hz (~4.194 MHz).
    const CLOCK_FREQUENCY: u32 = 1 << 22;

    /// Elapsed time per frame
    /// Intuitively it would make more sense to calculate this as `(1/f) * C`,
    /// but by doing the multiplication first, we avoid hitting nanosecond
    /// precision limits.
    ///
    /// Just to be sure this is right, here's some dimensional analysis:
    ///
    /// ```
    /// 1 * [C cy/fr] / [f cy/s]
    /// 1 * [C cy/fr] * [1/f s/cy]
    /// 1 * [C/f s/fr]
    /// C/f (s/fr)
    /// ```
    const FRAME_DURATION: Duration = Duration::from_secs(1)
        .checked_mul(Clock::CYCLES_PER_FRAME.0 as u32)
        .unwrap()
        .checked_div(Self::CLOCK_FREQUENCY)
        .unwrap();

    /// Initialize a new clock
    pub fn new() -> Self {
        Self {
            cycles: Cycles(0),
            frame_start: Instant::now(),
        }
    }

    /// Is the current tick the last of its frame?
    ///
    /// The first tick (`0`) is the *first* of its frame. The last tick of the
    /// frame `n` will be `n * CYCLES_PER_FRAME - 1`.
    pub fn is_frame_end(&self) -> bool {
        (self.cycles.0 + 1).is_multiple_of(Self::CYCLES_PER_FRAME.0)
    }

    /// Get the clock cycle count for the next end-of-frame cycle, starting at
    /// the given cycle
    ///
    /// If the given cycle is already the end of a frame, then the end of the
    /// *following* frame will be returned.
    pub fn next_frame_end(cycles: Cycles) -> Cycles {
        // +1 ensures the last cycle of a frame jumps to the next frame
        Cycles((cycles + 1).0.next_multiple_of(Self::CYCLES_PER_FRAME.0))
    }

    /// Get the number of cycles completed in the current frame
    pub fn cycles(&self) -> Cycles {
        self.cycles
    }

    /// Sleep until the end of the frame
    ///
    /// Ideally this would sleep once per tick, but the sleep function is way
    /// too imprecise for that.
    pub fn sleep(&mut self) {
        const SLEEP_INCREMENT: Duration = Duration::from_millis(1);

        let now = self.frame_start;
        let elapsed = now.elapsed();
        if elapsed < Self::FRAME_DURATION {
            let target = now + Self::FRAME_DURATION;
            // Sleep in 1ms increments to minimize the error. If we sleep for
            // the entire duration at once, the OS may sleep fortoo long.
            while Instant::now() + SLEEP_INCREMENT < target {
                thread::sleep(SLEEP_INCREMENT);
            }
            // Whatever's left <1ms is ignored
        } else {
            // Frame took too long, which means the component ticking took
            // longer than allowed. Unfortunately we can't make time go
            // backward (yet), so just log it and pray we speed up.
            warn!(
                "Slow frame: {elapsed:?} > {duration:?}",
                duration = Self::FRAME_DURATION
            );
        }
        self.frame_start = Instant::now();
    }

    /// Advance the clock one cycle
    pub fn tick(&mut self) {
        self.cycles += Cycles(1);
    }
}

/// Newtype for a number of clock cycles
///
/// This makes it clearer what a value is, instead of passing around a bare
/// integer everywhere. Every executed instruction returns this value so the CPU
/// can report how many cycles were consumed from the budget.
#[derive(Clone, Copy, Debug, Default, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Cycles(pub u64);

impl Cycles {
    /// Increment this value by 1
    pub fn incr(self) -> Cycles {
        self + Cycles(1)
    }
}

impl Display for Cycles {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Add for Cycles {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self(self.0 + rhs.0)
    }
}

impl Add<u64> for Cycles {
    type Output = Self;

    fn add(self, rhs: u64) -> Self::Output {
        Self(self.0 + rhs)
    }
}

impl AddAssign for Cycles {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

impl Sub for Cycles {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        Self(self.0 - rhs.0)
    }
}

impl Mul<u8> for Cycles {
    type Output = Self;

    fn mul(self, rhs: u8) -> Self::Output {
        Self(self.0 * u64::from(rhs))
    }
}
