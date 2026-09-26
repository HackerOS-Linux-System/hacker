use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::ptr;

use crate::{parse, parse_file, HackerFile};

// ─── Version tags returned by `hacker_version` ─────────────────────────────

/// Invalid handle / parse error.
pub const HACKER_VERSION_ERROR: i64 = 0;
/// Handle holds a `HackerFile::V1`.
pub const HACKER_VERSION_V1: i64 = 1;
/// Handle holds a `HackerFile::V2`.
pub const HACKER_VERSION_V2: i64 = 2;
/// Handle holds a `HackerFile::V3`.
pub const HACKER_VERSION_V3: i64 = 3;

// ─── Small internal helpers ─────────────────────────────────────────────────

/// Reads a possibly-null, caller-owned C string as a `&str`. Never panics
/// on invalid UTF-8 or a null pointer — both fall back to `""`, matching
/// H#'s own runtime contract for string parameters ("guaranteed non-null",
/// see `codegen.rs`'s `mark_param_nonnull` comment) as closely as a
/// foreign, possibly-misused pointer can.
unsafe fn cstr_in<'a>(ptr: *const c_char) -> &'a str {
    if ptr.is_null() {
        return "";
    }
    CStr::from_ptr(ptr).to_str().unwrap_or("")
}

/// Hands a fresh, NUL-terminated, heap-allocated copy of `s` to the caller.
/// Paired one-to-one with `hacker_free_string` — see the module-level
/// ownership note above. `CString::new` only fails if `s` itself contains
/// an interior NUL, which can't happen for any value this crate actually
/// returns (parsed `.hacker` field text), so the fallback is unreachable
/// in practice but kept instead of `.unwrap()` to guarantee this function
/// itself can never panic.
fn cstring_out(s: String) -> *mut c_char {
    CString::new(s).unwrap_or_default().into_raw()
}

/// Recovers the boxed `HackerFile` behind a handle without taking
/// ownership of it. `None` for the `0` sentinel — callers never get a
/// non-zero handle for anything else, so any other value is trusted to be
/// a live pointer this module itself produced.
unsafe fn handle_ref<'a>(handle: i64) -> Option<&'a HackerFile> {
    if handle == 0 {
        None
    } else {
        Some(&*(handle as *const HackerFile))
    }
}

// ─── Parsing / lifetime ─────────────────────────────────────────────────────

/// Parses `input` (a `.hacker`-format string) and returns an opaque
/// handle, or `0` if `input` isn't valid `.hacker` syntax at all
/// (`parse`'s only real failure mode — see `ParseError::MissingBrackets`/
/// `EmptyContent`; anything past the brackets always succeeds, falling
/// back to v1 per `hacker_parser::parse`'s own doc comment).
///
/// Free the returned handle with `hacker_free` once you're done with it.
#[no_mangle]
pub extern "C" fn hacker_parse(input: *const c_char) -> i64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let text = unsafe { cstr_in(input) };
        parse(text)
    }));
    match result {
        Ok(Ok(file)) => Box::into_raw(Box::new(file)) as i64,
        _ => 0,
    }
}

/// Reads and parses the `.hacker` file at `path`. Returns `0` on any I/O
/// error (missing file, permissions, ...) or parse failure — this
/// collapses `ParseError::IoError` and the syntax errors into the same
/// sentinel `hacker_parse` uses, since the flat ABI has no channel to
/// carry the distinction (or the error message) back across the boundary.
#[no_mangle]
pub extern "C" fn hacker_parse_file(path: *const c_char) -> i64 {
    let result = catch_unwind(AssertUnwindSafe(|| {
        let p = unsafe { cstr_in(path) };
        parse_file(p)
    }));
    match result {
        Ok(Ok(file)) => Box::into_raw(Box::new(file)) as i64,
        _ => 0,
    }
}

/// Frees a handle returned by `hacker_parse`/`hacker_parse_file`. `0` is a
/// no-op. Calling this twice on the same non-zero handle, or using the
/// handle again afterward, is undefined behavior (exactly the same
/// contract as `free`/`Box::from_raw` in C/Rust) — the flat ABI has no way
/// to enforce this from the H# side, so it's on the caller.
#[no_mangle]
pub extern "C" fn hacker_free(handle: i64) {
    if handle == 0 {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(Box::from_raw(handle as *mut HackerFile));
    }));
}

/// Frees a `string` returned by any accessor in this module
/// (`hacker_v1_content`, `hacker_v2_header`, `hacker_v3_*`, ...). `0`/null
/// is a no-op. See the module-level ownership note — every accessor
/// allocates an independent copy, so this must be called once per
/// accessor call, not once per handle.
#[no_mangle]
pub extern "C" fn hacker_free_string(s: *mut c_char) {
    if s.is_null() {
        return;
    }
    let _ = catch_unwind(AssertUnwindSafe(|| unsafe {
        drop(CString::from_raw(s));
    }));
}

// ─── Version tag ─────────────────────────────────────────────────────────────

/// Returns `HACKER_VERSION_V1`/`_V2`/`_V3` for the handle's actual variant,
/// or `HACKER_VERSION_ERROR` (`0`) for an invalid/`0` handle.
#[no_mangle]
pub extern "C" fn hacker_version(handle: i64) -> i64 {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V1(_)) => HACKER_VERSION_V1,
        Some(HackerFile::V2(_)) => HACKER_VERSION_V2,
        Some(HackerFile::V3(_)) => HACKER_VERSION_V3,
        None => HACKER_VERSION_ERROR,
    }
}

// ─── V1 accessors ────────────────────────────────────────────────────────────

/// The raw content string of a v1 file. Null if `handle` isn't a v1 file
/// (or is invalid).
#[no_mangle]
pub extern "C" fn hacker_v1_content(handle: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V1(v)) => cstring_out(v.content.clone()),
        _ => ptr::null_mut(),
    }
}

// ─── V2 accessors ────────────────────────────────────────────────────────────
//
// `HackerV2.sections` is a `HashMap<String, Vec<String>>`, which has no
// fixed-width ABI shape at all — so it's exposed index-by-index instead,
// the same "count, then index" convention C APIs use for any dynamically
// sized collection (`argc`/`argv`, `sqlite3_column_count`, ...). Section
// *names* are sorted before indexing so `hacker_v2_section_name` gives a
// stable, deterministic order across calls despite `HashMap` itself having
// none — important since a caller is expected to loop
// `0..hacker_v2_section_count(h)` and get the same name back every time.

/// The header text of a v2 file. Null if `handle` isn't a v2 file.
#[no_mangle]
pub extern "C" fn hacker_v2_header(handle: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V2(v)) => cstring_out(v.header.clone()),
        _ => ptr::null_mut(),
    }
}

/// Number of distinct sections in a v2 file. `0` if `handle` isn't a v2
/// file.
#[no_mangle]
pub extern "C" fn hacker_v2_section_count(handle: i64) -> i64 {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V2(v)) => v.sections.len() as i64,
        _ => 0,
    }
}

/// Sorted-order section name at `index` (`0 <= index < hacker_v2_section_count(handle)`).
/// Null if `handle` isn't a v2 file or `index` is out of range.
#[no_mangle]
pub extern "C" fn hacker_v2_section_name(handle: i64, index: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V2(v)) => {
            if index < 0 {
                return ptr::null_mut();
            }
            let mut names: Vec<&String> = v.sections.keys().collect();
            names.sort();
            match names.get(index as usize) {
                Some(name) => cstring_out((*name).clone()),
                None => ptr::null_mut(),
            }
        }
        _ => ptr::null_mut(),
    }
}

/// Number of `= value` entries under section `name`. `0` if `handle` isn't
/// a v2 file or has no such section.
#[no_mangle]
pub extern "C" fn hacker_v2_value_count(handle: i64, name: *const c_char) -> i64 {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V2(v)) => {
            let name = unsafe { cstr_in(name) };
            v.sections.get(name).map(|vals| vals.len() as i64).unwrap_or(0)
        }
        _ => 0,
    }
}

/// The value at `index` under section `name` (`0 <= index <
/// hacker_v2_value_count(handle, name)`). Null if `handle` isn't a v2
/// file, the section doesn't exist, or `index` is out of range.
#[no_mangle]
pub extern "C" fn hacker_v2_value_at(handle: i64, name: *const c_char, index: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V2(v)) => {
            if index < 0 {
                return ptr::null_mut();
            }
            let name = unsafe { cstr_in(name) };
            match v.sections.get(name).and_then(|vals| vals.get(index as usize)) {
                Some(val) => cstring_out(val.clone()),
                None => ptr::null_mut(),
            }
        }
        _ => ptr::null_mut(),
    }
}

// ─── V3 accessors ────────────────────────────────────────────────────────────

/// The `= header` field of a v3 file. Null if `handle` isn't a v3 file.
#[no_mangle]
pub extern "C" fn hacker_v3_header(handle: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V3(v)) => cstring_out(v.header.clone()),
        _ => ptr::null_mut(),
    }
}

/// The `=> type` field of a v3 file. Null if `handle` isn't a v3 file.
#[no_mangle]
pub extern "C" fn hacker_v3_type(handle: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V3(v)) => cstring_out(v.ty.clone()),
        _ => ptr::null_mut(),
    }
}

/// The `-> description` field of a v3 file. Null if `handle` isn't a v3
/// file.
#[no_mangle]
pub extern "C" fn hacker_v3_description(handle: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V3(v)) => cstring_out(v.description.clone()),
        _ => ptr::null_mut(),
    }
}

/// The `--> file` field of a v3 file. Null if `handle` isn't a v3 file.
#[no_mangle]
pub extern "C" fn hacker_v3_file(handle: i64) -> *mut c_char {
    match unsafe { handle_ref(handle) } {
        Some(HackerFile::V3(v)) => cstring_out(v.file.clone()),
        _ => ptr::null_mut(),
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────
//
// Plain Rust tests exercising the C ABI itself (raw pointers, handles,
// manual free calls) — the same shape an H# caller would drive it through,
// just without needing an H# toolchain to run `cargo test`.
#[cfg(test)]
mod tests {
    use super::*;

    fn to_c(s: &str) -> CString {
        CString::new(s).unwrap()
    }

    #[test]
    fn v1_roundtrip() {
        let input = to_c("[ 0.1 ]");
        let h = hacker_parse(input.as_ptr());
        assert_ne!(h, 0);
        assert_eq!(hacker_version(h), HACKER_VERSION_V1);

        let content = hacker_v1_content(h);
        assert!(!content.is_null());
        let s = unsafe { CStr::from_ptr(content) }.to_str().unwrap();
        assert_eq!(s, "0.1");

        hacker_free_string(content);
        hacker_free(h);
    }

    #[test]
    fn v2_roundtrip() {
        let input = to_c("[\nHeader\njadro:\n= xanmod\n= liquorix\n]");
        let h = hacker_parse(input.as_ptr());
        assert_ne!(h, 0);
        assert_eq!(hacker_version(h), HACKER_VERSION_V2);

        assert_eq!(hacker_v2_section_count(h), 1);
        let name_ptr = hacker_v2_section_name(h, 0);
        let name = unsafe { CStr::from_ptr(name_ptr) }.to_str().unwrap().to_string();
        assert_eq!(name, "jadro");
        hacker_free_string(name_ptr);

        let name_c = to_c("jadro");
        assert_eq!(hacker_v2_value_count(h, name_c.as_ptr()), 2);
        let v0 = hacker_v2_value_at(h, name_c.as_ptr(), 0);
        assert_eq!(unsafe { CStr::from_ptr(v0) }.to_str().unwrap(), "xanmod");
        hacker_free_string(v0);

        hacker_free(h);
    }

    #[test]
    fn v3_roundtrip() {
        let input = to_c("[\n= application\n=> gui\n-> aplication for cybersecurity\n--> main.hacker\n]");
        let h = hacker_parse(input.as_ptr());
        assert_ne!(h, 0);
        assert_eq!(hacker_version(h), HACKER_VERSION_V3);

        let ty = hacker_v3_type(h);
        assert_eq!(unsafe { CStr::from_ptr(ty) }.to_str().unwrap(), "gui");
        hacker_free_string(ty);

        hacker_free(h);
    }

    #[test]
    fn invalid_input_returns_zero_handle() {
        let input = to_c("not a hacker file");
        assert_eq!(hacker_parse(input.as_ptr()), 0);
    }

    #[test]
    fn null_handle_is_safe_everywhere() {
        assert_eq!(hacker_version(0), HACKER_VERSION_ERROR);
        assert!(hacker_v1_content(0).is_null());
        assert!(hacker_v2_header(0).is_null());
        assert!(hacker_v3_header(0).is_null());
        hacker_free(0);
        hacker_free_string(ptr::null_mut());
    }
}
