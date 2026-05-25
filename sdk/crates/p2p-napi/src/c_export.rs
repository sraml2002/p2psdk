//! C ABI exports — thin wrappers for non-NAPI consumers (e.g. Rust Demo App).
//!
//! Provides `#[no_mangle] extern "C"` functions that delegate to client_napi.
//! C callback function pointers are stored in atomics; fire_c_* helpers
//! are called alongside the existing TSFN callbacks in napi_bridge.rs.

use std::ffi::{c_char, c_int};
use std::sync::atomic::{AtomicPtr, Ordering};

// ── C Callback types ───────────────────────────────────────────────

type CbState = extern "C" fn(*const c_char);
type CbData = extern "C" fn(*const u8, usize);
type CbLog = extern "C" fn(*const c_char);

static CB_STATE: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static CB_DATA: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());
static CB_LOG: AtomicPtr<()> = AtomicPtr::new(std::ptr::null_mut());

// ── C callback fire helpers — called from napi_bridge fire_* ───────

pub fn fire_c_state(state: &str) {
    let ptr = CB_STATE.load(Ordering::Acquire);
    if ptr.is_null() {
        return;
    }
    let cb: CbState = unsafe { std::mem::transmute(ptr) };
    let c_str = format!("{state}\0");
    cb(c_str.as_ptr() as *const c_char);
}

pub fn fire_c_data(data: &[u8]) {
    let ptr = CB_DATA.load(Ordering::Acquire);
    if ptr.is_null() {
        return;
    }
    let cb: CbData = unsafe { std::mem::transmute(ptr) };
    cb(data.as_ptr(), data.len());
}

pub fn fire_c_log(msg: &str) {
    let ptr = CB_LOG.load(Ordering::Acquire);
    if ptr.is_null() {
        return;
    }
    let cb: CbLog = unsafe { std::mem::transmute(ptr) };
    let c_str = format!("{msg}\0");
    cb(c_str.as_ptr() as *const c_char);
}

// ── C ABI exports ──────────────────────────────────────────────────

/// Register C callback function pointers. Call before ppsdk_init.
#[no_mangle]
pub extern "C" fn ppsdk_register_callbacks(
    on_state: CbState,
    on_data: CbData,
    on_log: CbLog,
) -> c_int {
    CB_STATE.store(on_state as *mut (), Ordering::Release);
    CB_DATA.store(on_data as *mut (), Ordering::Release);
    CB_LOG.store(on_log as *mut (), Ordering::Release);
    0
}

/// Initialize SDK with JSON config: {"idsUrl":"...", "natUrl":"..."}
#[no_mangle]
pub extern "C" fn ppsdk_init(config_json: *const c_char) -> c_int {
    let config = match c_str_to_string(config_json) {
        Some(s) => s,
        None => return -1,
    };
    crate::client_napi::init(&config)
}

/// Register device to IDS. Returns JSON string (free with ppsdk_free_string).
#[no_mangle]
pub extern "C" fn ppsdk_register_ids(
    app_id: *const c_char,
    user_id: *const c_char,
    odid: *const c_char,
    push_token: *const c_char,
) -> *mut c_char {
    let (Some(app_id), Some(user_id), Some(odid), Some(push_token)) = (
        c_str_to_string(app_id),
        c_str_to_string(user_id),
        c_str_to_string(odid),
        c_str_to_string(push_token),
    ) else {
        return std::ptr::null_mut();
    };
    let result = crate::client_napi::register_ids(&app_id, &user_id, &odid, &push_token);
    string_to_c_ptr(result)
}

/// Query peer IDS. Returns JSON string (free with ppsdk_free_string).
#[no_mangle]
pub extern "C" fn ppsdk_query_ids(
    app_id: *const c_char,
    user_id: *const c_char,
) -> *mut c_char {
    let (Some(app_id), Some(user_id)) =
        (c_str_to_string(app_id), c_str_to_string(user_id))
    else {
        return std::ptr::null_mut();
    };
    let result = crate::client_napi::query_ids(&app_id, &user_id);
    string_to_c_ptr(result)
}

/// One-click connect: token → gather candidates → SDP negotiate.
/// Results are delivered via on_state / on_data callbacks.
#[no_mangle]
pub extern "C" fn ppsdk_connect(
    peer_id: *const c_char,
    odid: *const c_char,
) -> c_int {
    let (Some(peer_id), Some(odid)) =
        (c_str_to_string(peer_id), c_str_to_string(odid))
    else {
        return -1;
    };
    crate::client_napi::connect(&peer_id, &odid, false, 30)
}

/// Send text message.
#[no_mangle]
pub extern "C" fn ppsdk_send_text(text: *const c_char) -> c_int {
    let Some(text) = c_str_to_string(text) else {
        return -1;
    };
    crate::client_napi::send_text(&text)
}

/// Send binary data.
#[no_mangle]
pub extern "C" fn ppsdk_send(data: *const u8, len: usize) -> c_int {
    if data.is_null() || len == 0 {
        return -1;
    }
    let slice = unsafe { std::slice::from_raw_parts(data, len) };
    crate::client_napi::send(slice)
}

/// Close connection and release resources.
#[no_mangle]
pub extern "C" fn ppsdk_close() -> c_int {
    crate::client_napi::close()
}

/// Free string returned by ppsdk_register_ids / ppsdk_query_ids.
#[no_mangle]
pub extern "C" fn ppsdk_free_string(s: *mut c_char) {
    if !s.is_null() {
        unsafe {
            let _ = std::ffi::CString::from_raw(s);
        }
    }
}

// ── Helpers ────────────────────────────────────────────────────────

fn c_str_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { std::ffi::CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

fn string_to_c_ptr(s: String) -> *mut c_char {
    std::ffi::CString::new(s).unwrap_or_default().into_raw()
}
