use std::{
    fmt::{self, Debug, Display},
    marker::PhantomData,
};

/// Index of a single bit in a byte
///
/// Value can be `0-7`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bit(pub u8);

impl Bit {
    /// List of all bit index 0-7
    const ALL: &[Self] = &[
        Self(0),
        Self(1),
        Self(2),
        Self(3),
        Self(4),
        Self(5),
        Self(6),
        Self(7),
    ];

    /// Get the value of a bit from a byte as a bool
    pub fn get(self, bits: u8) -> bool {
        bits & (0b1 << self.0) > 0
    }

    /// Set the value of a bit in a byte, returning the new byte
    #[must_use]
    pub fn set(self, bits: u8, flag: bool) -> u8 {
        let new = u8::from(flag) << self.0;
        (bits | new) & new
    }
}

/// A trait for structs of booleans that are stored as bitflags in a `u8`
///
/// The idea is you define your flags as a struct of booleans, then implement
/// this trait to map booleans to bits. This trait and [FlagBits] take care of
/// converting to `u8`, allowing you to store the flags as a byte for easy
/// runtime representation and interop, but semantically you can work with it
/// as a set of bools.
///
/// This is an alternative to the `bitflags` crate, because I think it's pretty
/// clunky to use.
pub trait Flags: Default {
    /// Get the boolean value of a flag based on its bit index
    ///
    /// This **will** be called for all bits 0-7. If the bit index isn't used,
    /// return `false`.
    fn get_bit(&self, bit: Bit) -> bool;

    /// Set the boolean value of a flag based on its bit index
    ///
    /// This **will** be called for all bits 0-7. If the bit index isn't used,
    /// do nothing.
    fn set_bit(&mut self, bit: Bit, value: bool);

    /// Read individual flags from the bits of the byte
    fn from_bits(bits: FlagBits<Self>) -> Self {
        let mut flags = Self::default();
        for bit in Bit::ALL {
            let flag = bit.get(bits.value);
            flags.set_bit(*bit, flag);
        }
        flags
    }

    /// Convert individual flags into bitflags
    fn into_bits(self) -> FlagBits<Self> {
        let mut bits = 0;
        for bit in Bit::ALL {
            let flag = self.get_bit(*bit);
            bits = bit.set(bits, flag);
        }
        FlagBits::new(bits)
    }
}

/// Byte representation of a [Flags] implementation
///
/// This is how flags should be represented in memory. [Flags::from_bits] and
/// [Flags::into_bits] to convert to/from this type.
#[derive(Clone, Copy, Default)]
pub struct FlagBits<T> {
    value: u8,
    ty: PhantomData<T>,
}

impl<T: Flags> FlagBits<T> {
    pub fn new(value: u8) -> Self {
        Self {
            value,
            ty: PhantomData,
        }
    }

    /// Get a mutable reference to the inner byte value
    #[cfg(test)]
    pub fn value_mut(&mut self) -> &mut u8 {
        &mut self.value
    }
}

impl<T> Debug for FlagBits<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&BytesDisplay::binary(&[self.value]), f)
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
