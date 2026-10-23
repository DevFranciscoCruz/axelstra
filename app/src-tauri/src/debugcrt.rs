//! Debug builds on Windows (MSVC): see `build.rs`. The static sherpa-onnx libs and `msvcrtd.lib` reference the debug
//! heap entry points, which only exist in the debug CRT that the test exe no longer links. Plain heap calls stand in.
use std::ffi::{c_char, c_int, c_void};

extern "C" {
    fn malloc(size: usize) -> *mut c_void;
    fn free(ptr: *mut c_void);
}

#[no_mangle]
pub unsafe extern "C" fn _malloc_dbg(size: usize, _block_use: c_int, _file: *const c_char, _line: c_int) -> *mut c_void {
    malloc(size)
}

#[no_mangle]
pub unsafe extern "C" fn _free_dbg(ptr: *mut c_void, _block_use: c_int) {
    free(ptr)
}
