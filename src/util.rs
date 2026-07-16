use std::{
    fmt::{self, Debug, Display},
    marker::PhantomData,
    ops::{BitAnd, BitOr, Deref, DerefMut, Not},
};

/// Index of a single bit in a byte
///
/// Value can be `0-7`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bit(pub u8);

impl Bit {
    /// Get the value of a bit from a byte as a bool
    pub fn get(self, bits: u8) -> bool {
        bits & self.mask() > 0
    }

    /// Set the value of a bit in a byte, returning the new byte
    #[must_use]
    pub fn set(self, bits: u8, flag: bool) -> u8 {
        let new = u8::from(flag) << self.0;
        (bits | new) & new
    }

    /// Get a [Mask] for this single bit
    pub const fn mask(self) -> Mask {
        Mask(0b1 << self.0)
    }
}

/// Newtype for a bitmask
///
/// There's a lot of `u8`s floating around in this file, so this helps keep them
/// all straight.
#[derive(Clone, Copy)]
pub struct Mask(u8);

impl Mask {
    /// Mask with no bits
    pub const ZERO: Self = Self(0);
    /// Mask for bits 1-0
    pub const M10: Self = Self(0b0000_0011);
    /// Mask for bits 2-0
    pub const M210: Self = Self(0b0000_0111);
    /// Mask for bits 4-3
    pub const M43: Self = Self(0b0001_1000);
    /// Mask for bits 5-4
    pub const M54: Self = Self(0b0011_0000);
    /// Mask for bits 5-3
    pub const M543: Self = Self(0b0011_1000);

    pub fn new(mask: u8) -> Self {
        Self(mask)
    }

    /// Mask out bits from the given value, then right-shift to puts those bits
    /// in the right-most place
    ///
    /// ```
    /// // Mask 0b1010_1010 to 0b0010_0000, then reduce to 0b10
    /// assert_eq!(Mask::new(0b0011_0000).reduced(0b1010_1010), 0b10);
    /// ```
    pub fn reduced(self, value: u8) -> u8 {
        (value & self) >> self.shift()
    }

    /// Get the number of bits required to right-shift this mask to the least
    /// significant bits
    pub fn shift(self) -> u32 {
        self.0.trailing_zeros()
    }
}

impl Debug for Mask {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Mask(0b{:0>8b})", self.0)
    }
}

impl Not for Mask {
    type Output = Self;

    fn not(self) -> Self::Output {
        Self(!self.0)
    }
}

impl BitAnd<Mask> for u8 {
    type Output = u8;

    fn bitand(self, rhs: Mask) -> Self::Output {
        self & rhs.0
    }
}

impl BitAnd for Mask {
    type Output = Self;

    fn bitand(self, rhs: Self) -> Self::Output {
        Self(self.0 & rhs.0)
    }
}

impl BitOr for Mask {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// A trait for types that can be packed into a single byte
///
/// This is used for structs that pack multiple values into one byte, but can
/// also be used for any value that is represented in bits. Typically a
/// `BitPack` struct is composed of various `BitPack` fields such as bools and
/// enums. Each field can be either one or multiple bits.
///
/// Do **not** implement this manually. Use [impl_bit_pack] for enums and
/// structs.
pub trait BitPack: Sized {
    /// Convert from bits to this value
    fn from_bits(bits: u8) -> Self;

    /// Convert this value to bits
    fn to_bits(&self) -> u8;

    /// Convert this value to bits, then wrap in [PackedBits]
    fn pack(self) -> PackedBits<Self> {
        PackedBits::new(self.to_bits())
    }
}

impl BitPack for bool {
    fn from_bits(bits: u8) -> Self {
        (bits & 0b1) == 1
    }

    fn to_bits(&self) -> u8 {
        (*self).into()
    }
}

/// Implement [BitPack] for structs and enums
///
/// For structs, this maps each field to a specific bit mask. The conversions
/// to/from bits are implemented automatically based on that mapping.
///
/// For enums, it maps each variant to a static byte value. Supported for unit
/// enums only.
///
/// It's impossible to forget a field/variant with this implementation because
/// the generated code will produce a compile error.
macro_rules! impl_bit_pack {
    (struct $type:ty; $($mask:expr => $field:ident),* $(,)?) => {
        impl $crate::util::BitPack for $type {
            fn from_bits(bits: u8) -> Self {
                Self {
                    $($field: $crate::util::BitPack::from_bits(
                        $mask.reduced(bits)
                    ),)*
                }
            }

            fn to_bits(&self) -> u8 {
                let mut bits = 0;
                $(bits |= self.$field.to_bits() << $mask.shift();)*
                bits
            }
        }
    };
    (enum $type:ty; $($value:literal => $variant:ident),* $(,)?) => {
        impl $crate::util::BitPack for $type {
            fn from_bits(bits: u8) -> Self {
                match bits {
                    $($value => Self::$variant,)*
                    _ => unreachable!("TODO"),
                }
            }

            fn to_bits(&self) -> u8 {
                match self {
                    $(Self::$variant => $value,)*
                }
            }
        }
    };
}
pub(crate) use impl_bit_pack;

/// A byte associated with a [BitPack]-implementing type
///
/// At runtime this is just the packed byte, but it carries an associated type
/// so it can be unpacked easily with [Self::unpack].
pub struct PackedBits<T> {
    value: u8,
    ty: PhantomData<T>,
}

impl<T: BitPack> PackedBits<T> {
    pub fn new(value: u8) -> Self {
        Self {
            value,
            ty: PhantomData,
        }
    }

    /// Unpack a byte value into a `T` using its [BitPack] implementation
    pub fn unpack(self) -> T {
        T::from_bits(self.value)
    }
}

impl<T> Clone for PackedBits<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for PackedBits<T> {}

impl<T> Debug for PackedBits<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        Debug::fmt(&BytesDisplay::binary(&[self.value]), f)
    }
}

impl<T> Default for PackedBits<T> {
    fn default() -> Self {
        Self {
            value: 0,
            ty: PhantomData,
        }
    }
}

impl<T> Deref for PackedBits<T> {
    type Target = u8;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T> DerefMut for PackedBits<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
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
