use std::fmt::{self, Debug, Display};

/// Index of a single bit in a byte
///
/// Value can be `0-7`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bit(pub u8);

impl Bit {
    /// Get the value of a bit from a byte as a bool
    pub fn get(self, value: u8) -> bool {
        value & (0b1 << self.0) > 0
    }

    /// Convert a boolean flag to a 0/1 in this bit position
    pub fn bit(self, value: bool) -> u8 {
        u8::from(value) << self.0
    }

    /// Set the value of a bit in a byte
    pub fn set(self, value: u8, bit_value: bool) -> u8 {
        let new = u8::from(bit_value) << self.0;
        (value | new) & new
    }
}

/// Wrapper to pretty print a byte slice
///
/// By default this will print a truncated slice, up to 8 bytes. To print the
/// whole thing, enable the alter display flag (`#`);
pub struct BytesDisplay<'a> {
    bytes: &'a [u8],
    mode: BytesDisplayMode,
}

impl<'a> BytesDisplay<'a> {
    /// Display bytes as binary
    pub fn binary(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            mode: BytesDisplayMode::Binary,
        }
    }

    /// Display bytes as hexadecimal
    pub fn hex(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            mode: BytesDisplayMode::Hex,
        }
    }
}

impl Debug for BytesDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Display::fmt(&self, f) // Defer to Display
    }
}

impl Display for BytesDisplay<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        const MAX: usize = 8;

        let len = if f.alternate() {
            self.bytes.len()
        } else {
            self.bytes.len().min(MAX)
        };
        let bytes = &self.bytes[..len];

        for (i, byte) in bytes.iter().enumerate() {
            if i > 0 {
                write!(f, " ")?;
            }
            match self.mode {
                BytesDisplayMode::Binary => write!(f, "{byte:0>8b}")?,
                BytesDisplayMode::Hex => write!(f, "{byte:0>2x}")?,
            }
        }

        let hidden = self.bytes.len() - len;
        if hidden > 0 {
            write!(f, " <+{hidden} bytes>")?;
        }

        Ok(())
    }
}

/// How to display bytes in [BytesDisplay]
enum BytesDisplayMode {
    Binary,
    Hex,
}
