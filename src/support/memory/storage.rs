//! Long-lived buffers in PSRAM.
//!
//! Both helpers allocate once and never free: they are for buffers that live
//! as long as the device runs, such as a framebuffer or a camera frame.
//! Anything that talks to hardware directly (DMA descriptors, task stacks,
//! synchronization objects) stays in internal RAM.

use allocator_api2::{boxed::Box, vec::Vec};

use super::psram;

/// A slice of `len` copies of `value` in PSRAM.
pub fn leaked_slice<T: Clone + 'static>(len: usize, value: T) -> &'static mut [T] {
    let mut storage = Vec::with_capacity_in(len, psram::heap());
    storage.resize(len, value);
    storage.leak()
}

/// One value built by `init` directly in PSRAM, for objects too large for a
/// task's stack.
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
