//! [MemoryView] implementation
//!
//! This is hidden in a submodule because it does super duper scary `unsafe`
//! stuff and I really want to hide the internals.
use crate::emu::{Address, memory::AddressRange};
use std::{mem, slice};

/// A temporary view into a block of memory
///
/// The memory bus uses this to treat arbitrary blocks of memory as raw bytes.
/// This encapsulates safety guarantees. It also provides modal functionality
/// for memory that is dynamically nullable.
#[derive(Debug)]
pub struct MemoryView<'a> {
    /// Underlying memory value (typically a slice)
    value: Value<'a>,
    /// Range of memory addresses covered by this view
    ///
    /// Any operation attempted on an address outside this range will panic.
    range: AddressRange,
}

impl<'a> MemoryView<'a> {
    /// Create a byte view into a slice of some type `T`
    ///
    /// This allows a range of the memory bus to be backed by some arbitrary
    /// memory, as long as that memory has a consistent and correcy memory
    /// layout.
    ///
    /// Panics if `range.len()` does not equal the **byte length** of `slice`.
    pub fn from_slice<T>(slice: &'a [T], range: AddressRange) -> Self {
        let byte_len = mem::size_of_val(slice);
        // Make sure the length of the address range matches the byte length
        // of the slice
        debug_assert_eq!(
            byte_len,
            range.len(),
            "Slice byte length must match address range length",
        );
        // SAFETY:
        // - Pointer is valid because it was just passed in. We carry the
        //   lifetime over so it remains valid for the lifetime of this struct
        // - Length is correct because it's calculated from the slice above
        let bytes = unsafe {
            slice::from_raw_parts(slice.as_ptr().cast::<u8>(), byte_len)
        };
        Self {
            value: Value::Slice(bytes),
            range,
        }
    }

    /// Create a view to null memory
    ///
    /// This is not backed by any real memory. [get](Self::get) will always
    /// return `0` and [set](Self::set) does nothing.
    pub fn null(range: AddressRange) -> Self {
        Self {
            value: Value::Null,
            range,
        }
    }

    /// Get the value of a single byte
    ///
    /// Panics if `address` is not in this view's range.
    pub fn get(&self, address: Address) -> u8 {
        match self.value {
            Value::Null => 0,
            Value::Slice(bytes) => {
                let offset = self.range.offset(address);
                // SAFETY: self.range.len() == self.bytes.len()
                bytes[offset]
            }
        }
    }

    /// Set the value of a single byte
    ///
    /// Panics if `address` is not in this view's range.
    pub fn set(&mut self, address: Address, value: u8) {
        match self.value {
            Value::Null => {}
            Value::Slice(bytes) => {
                let offset = self.range.offset(address);

                // Cast to &mut
                // SAFETY: TODO (it's not really safe is it...)
                // Maybe mutability can be parameterized somehow?
                let bytes = unsafe {
                    slice::from_raw_parts_mut(
                        bytes.as_ptr().cast_mut(),
                        bytes.len(),
                    )
                };
                // SAFETY: self.range.len() == self.bytes.len()
                bytes[offset] = value;
            }
        }
    }
}

#[derive(Debug)]
enum Value<'a> {
    /// Memory is always 0 and writes do nothing
    Null,
    /// View is backed by a byte slice
    ///
    /// Invariant: length of this slice equals the length of the parent's range
    Slice(&'a [u8]),
}
