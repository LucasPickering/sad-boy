//! Game Boy clock emulation

use std::{
    cell::Cell,
    cmp::Ordering,
    fmt::{self, Display},
    future,
    ops::{Add, AddAssign, Sub},
    task::Poll,
    thread,
    time::{Duration, Instant},
};
use tracing::warn;

/// Number of dots (clock cycles) in a single frame
///
/// - https://gbdev.io/pandocs/Rendering.html
/// - https://josaphat.co/posts/gameboy-emulator/
const CYCLES_PER_FRAME: Cycles = Cycles(70224);
/// Number of clock cycles per second (Hz)
///
/// The clock frequency is 2^22 Hz (~4.194 MHz).
const CLOCK_FREQUENCY: u32 = 1 << 22;
/// Elapsed time per frame
/// Intuitively it would make more sense to calculate this as `(1/f) * C`, but
/// by doing the multiplication first, we avoid hitting nanosecond precision
/// limits.
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
    .checked_mul(CYCLES_PER_FRAME.0 as u32)
    .unwrap()
    .checked_div(CLOCK_FREQUENCY)
    .unwrap();
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
    /// Moment when the final tick of the previous frame was completed
    frame_start: Cell<Instant>,
}

impl Clock {
    /// Initialize a new clock
    pub fn new() -> Self {
        Self {
            cycles: Cell::default(),
            frame_start: Instant::now().into(),
        }
    }

    /// Is the current tick the first of its frame?
    pub fn is_frame_start(&self) -> bool {
        self.cycles.get().0.is_multiple_of(CYCLES_PER_FRAME.0)
    }

    /// Is the current tick the last of its frame?
    pub fn is_frame_end(&self) -> bool {
        (self.cycles.get().0 + 1).is_multiple_of(CYCLES_PER_FRAME.0)
    }

    /// Get the number of cycles completed in the current frame
    pub fn cycles(&self) -> Cycles {
        self.cycles.get()
    }

    /// Sleep until the end of the frame
    ///
    /// Ideally this would sleep once per tick, but the sleep function is way
    /// too imprecise for that.
    pub fn sleep(&self) {
        const SLEEP_INCREMENT: Duration = Duration::from_millis(1);

        let now = self.frame_start.get();
        let elapsed = now.elapsed();
        if elapsed < FRAME_DURATION {
            let target = now + FRAME_DURATION;
            // Sleep in 1ms increments to minimize the error. If we sleep for
            // the entire duration at once, the OS may sleep fortoo long.
            while Instant::now() + SLEEP_INCREMENT < target {
                thread::sleep(SLEEP_INCREMENT);
            }
            // Whatever's left <1ms is ignored
        } else {
            // Frame took too long, which means the future polling took
            // longer than allowed. Unfortunately we can't make time go
            // backward (yet), so just log it and pray we speed up.
            warn!("Slow frame: {elapsed:?} > {FRAME_DURATION:?}");
        }
        self.frame_start.set(Instant::now());
    }

    /// Advance the clock one cycle
    pub fn tick(&self) {
        self.cycles.update(|cycles| cycles + Cycles(1));
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
        tracing::error!(?current, ?target, "init wait"); // TODO
        future::poll_fn(move |_| {
            let current = self.cycles.get();
            match current.cmp(&target) {
                Ordering::Less => Poll::Pending,
                Ordering::Equal => Poll::Ready(()),
                Ordering::Greater => {
                    // This *should* be impossible because every future gets
                    // polled on every clock cycle. Missing cycles could affect
                    // semantics
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

impl AddAssign for Cycles {
    fn add_assign(&mut self, rhs: Self) {
        self.0 += rhs.0;
    }
}

// TODO delete?
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
