use std::{ops, ptr, slice};

use super::ffi::*;

/// A single captured frame. The underlying IOSurface is locked for reading
/// for the whole lifetime of this `Frame`, so `deref` can hand out the
/// surface's plane-0 (BGRA) memory directly — no per-frame copy. The lock is
/// released in `Drop`, after which the memory must not be touched again.
pub struct Frame {
    surface: IOSurfaceRef,
    data: &'static [u8],
    stride: usize,
}

impl Frame {
    /// # Safety
    /// `surface` must be a valid IOSurface delivered by the display stream,
    /// and `height` its pixel height. The returned `Frame` keeps the surface
    /// retained and read-locked until dropped.
    pub unsafe fn new(surface: IOSurfaceRef, height: usize) -> Frame {
        CFRetain(surface);
        IOSurfaceIncrementUseCount(surface);
        IOSurfaceLock(surface, SURFACE_LOCK_READ_ONLY, ptr::null_mut());

        let stride = IOSurfaceGetBytesPerRowOfPlane(surface, 0);
        let base = IOSurfaceGetBaseAddressOfPlane(surface, 0) as *const u8;
        // Valid while the surface stays locked, i.e. for this Frame's lifetime.
        let data = slice::from_raw_parts(base, stride * height);

        Frame {
            surface,
            data,
            stride,
        }
    }

    #[inline]
    pub fn stride(&self) -> usize {
        self.stride
    }
}

impl ops::Deref for Frame {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        self.data
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe {
            IOSurfaceUnlock(self.surface, SURFACE_LOCK_READ_ONLY, ptr::null_mut());
            IOSurfaceDecrementUseCount(self.surface);
            CFRelease(self.surface);
        }
    }
}
