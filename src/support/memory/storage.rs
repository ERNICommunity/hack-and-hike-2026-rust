//! Long-lived buffers in PSRAM.
//!
//! Both helpers allocate once and never free: they are for buffers that live
//! as long as the device runs, such as a canvas or a camera frame. Because
//! nothing is ever freed, the returned references are `'static` and can be
//! stored in any struct.
//!
//! Anything that talks to hardware directly (DMA descriptors, task stacks,
//! synchronization objects) stays in internal RAM.
//!
//! ```ignore
//! let samples: &'static mut [i16] = storage::leaked_slice(16_000, 0);
//! ```

use allocator_api2::{boxed::Box, vec::Vec};

use super::psram;

/// A slice of `len` copies of `value` in PSRAM.
///
/// # Panics
///
/// When PSRAM is exhausted.
pub fn leaked_slice<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let mut storage = Vec::with_capacity_in(len, psram::heap());
    storage.resize(len, value);
    storage.leak()
}

/// One value, built by `init` and moved into PSRAM.
///
/// Use it for objects that should not live in a struct on a task's stack
/// because they are large. `init` still builds the value on the calling
/// stack before it moves, so the caller's stack needs room for it once.
///
/// # Panics
///
/// When PSRAM is exhausted.
pub fn leaked_value<T: 'static>(init: impl FnOnce() -> T) -> &'static mut T {
    let mut storage = Box::<T, _>::new_uninit_in(psram::heap());
    // SAFETY: `storage` owns one properly aligned, uninitialized allocation for
    // exactly one `T`. The value is written exactly once and no initialized
    // reference exists before that write.
    unsafe {
        storage.as_mut_ptr().write(init());
        Box::leak(storage.assume_init())
    }
}
