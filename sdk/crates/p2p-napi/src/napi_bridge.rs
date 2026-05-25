//! NAPI bridge — raw FFI module registration (VFSApi style).
//!
//! Uses hand-written NAPI FFI + .init_array for module registration.
//! No napi-ohos crate dependency.

use std::ffi::{c_char, c_int, c_uint, c_void};
use std::ptr;
use std::sync::atomic::{AtomicPtr, Ordering};

use crate::hilog;

// ── NAPI type aliases ──────────────────────────────────────────────
type NapiEnv = *mut c_void;
type NapiValue = *mut c_void;
type NapiCallbackInfo = *mut c_void;
type NapiStatus = c_int;
type NapiCallback = extern "C" fn(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue;
type NapiThreadsafeFunction = *mut c_void;
type NapiFinalize = Option<extern "C" fn(NapiEnv, *mut c_void, *mut c_void)>;

#[repr(C)]
struct NapiPropertyDescriptor {
    utf8name: *const c_char,
    name: NapiValue,
    method: Option<NapiCallback>,
    getter: Option<NapiCallback>,
    setter: Option<NapiCallback>,
    value: NapiValue,
    attributes: u32,
    data: *mut c_void,
}

#[repr(C)]
struct NapiModule {
    nm_version: c_int,
    nm_flags: c_uint,
    nm_filename: *const c_char,
    nm_register_func: Option<extern "C" fn(NapiEnv, NapiValue) -> NapiValue>,
    nm_modname: *const c_char,
    nm_priv: *mut c_void,
    reserved: [*mut c_void; 4],
}

unsafe impl Sync for NapiModule {}

// ── FFI to libace_napi.z.so ────────────────────────────────────────
extern "C" {
    fn napi_module_register(module: *const NapiModule);
    fn napi_define_properties(
        env: NapiEnv,
        object: NapiValue,
        property_count: usize,
        properties: *const NapiPropertyDescriptor,
    ) -> NapiStatus;
    fn napi_get_cb_info(
        env: NapiEnv,
        cbinfo: NapiCallbackInfo,
        argc: *mut usize,
        argv: *mut NapiValue,
        this_arg: *mut NapiValue,
        data: *mut *mut c_void,
    ) -> NapiStatus;
    fn napi_get_value_string_utf8(
        env: NapiEnv,
        value: NapiValue,
        buf: *mut c_char,
        bufsize: usize,
        result: *mut usize,
    ) -> NapiStatus;
    fn napi_create_string_utf8(
        env: NapiEnv,
        str_: *const c_char,
        length: usize,
        result: *mut NapiValue,
    ) -> NapiStatus;
    fn napi_create_int32(env: NapiEnv, value: c_int, result: *mut NapiValue) -> NapiStatus;
    fn napi_get_undefined(env: NapiEnv, result: *mut NapiValue) -> NapiStatus;
    fn napi_call_function(
        env: NapiEnv,
        recv: NapiValue,
        func: NapiValue,
        argc: usize,
        argv: *const NapiValue,
        result: *mut NapiValue,
    ) -> NapiStatus;
    fn napi_create_threadsafe_function(
        env: NapiEnv,
        func: NapiValue,
        async_resource: NapiValue,
        async_resource_name: NapiValue,
        max_queue_size: usize,
        initial_thread_count: usize,
        thread_finalize_data: *mut c_void,
        thread_finalize_cb: NapiFinalize,
        context: *mut c_void,
        call_js_cb: Option<extern "C" fn(NapiEnv, NapiValue, *mut c_void, *mut c_void)>,
        result: *mut NapiThreadsafeFunction,
    ) -> NapiStatus;
    fn napi_call_threadsafe_function(
        func: NapiThreadsafeFunction,
        data: *mut c_void,
        is_blocking: c_int,
    ) -> NapiStatus;
    fn napi_release_threadsafe_function(
        func: NapiThreadsafeFunction,
        mode: c_int,
    ) -> NapiStatus;
    fn napi_create_arraybuffer(
        env: NapiEnv,
        byte_length: usize,
        data: *mut *mut c_void,
        result: *mut NapiValue,
    ) -> NapiStatus;
    fn napi_get_arraybuffer_info(
        env: NapiEnv,
        value: NapiValue,
        data: *mut *mut c_void,
        byte_length: *mut usize,
    ) -> NapiStatus;
    fn napi_create_object(env: NapiEnv, result: *mut NapiValue) -> NapiStatus;
    fn napi_set_named_property(
        env: NapiEnv,
        object: NapiValue,
        key: *const c_char,
        value: NapiValue,
    ) -> NapiStatus;
    fn napi_create_array(env: NapiEnv, result: *mut NapiValue) -> NapiStatus;
    fn napi_set_element(
        env: NapiEnv,
        object: NapiValue,
        index: u32,
        value: NapiValue,
    ) -> NapiStatus;
    fn napi_create_uint32(env: NapiEnv, value: u32, result: *mut NapiValue) -> NapiStatus;
    fn napi_get_boolean(env: NapiEnv, value: bool, result: *mut NapiValue) -> NapiStatus;
    fn napi_get_value_bool(env: NapiEnv, value: NapiValue, result: *mut bool) -> NapiStatus;
    fn napi_get_value_uint32(env: NapiEnv, value: NapiValue, result: *mut u32) -> NapiStatus;
}

// ── Helpers ────────────────────────────────────────────────────────
unsafe fn get_cb_args(env: NapiEnv, info: NapiCallbackInfo, max_args: usize) -> (usize, Vec<NapiValue>) {
    let mut argc: usize = max_args;
    let mut args: Vec<NapiValue> = vec![ptr::null_mut(); max_args];
    napi_get_cb_info(env, info, &mut argc, args.as_mut_ptr(), ptr::null_mut(), ptr::null_mut());
    (argc, args)
}

fn read_napi_string(env: NapiEnv, val: NapiValue) -> Option<String> {
    let mut len: usize = 0;
    unsafe { napi_get_value_string_utf8(env, val, ptr::null_mut(), 0, &mut len); }
    if len == 0 {
        return Some(String::new());
    }
    let mut buf: Vec<u8> = vec![0u8; len + 1];
    unsafe {
        napi_get_value_string_utf8(env, val, buf.as_mut_ptr() as *mut c_char, len + 1, &mut len);
    }
    buf.truncate(len);
    String::from_utf8(buf).ok()
}

fn return_int32(env: NapiEnv, value: c_int) -> NapiValue {
    let mut result: NapiValue = ptr::null_mut();
    unsafe { napi_create_int32(env, value, &mut result); }
    result
}

fn return_string(env: NapiEnv, s: &str) -> NapiValue {
    let mut result: NapiValue = ptr::null_mut();
    unsafe {
        napi_create_string_utf8(env, s.as_ptr() as *const c_char, s.len(), &mut result);
    }
    result
}

fn read_napi_arraybuffer(env: NapiEnv, val: NapiValue) -> Option<Vec<u8>> {
    let mut data_ptr: *mut c_void = ptr::null_mut();
    let mut byte_len: usize = 0;
    unsafe {
        let status = napi_get_arraybuffer_info(env, val, &mut data_ptr, &mut byte_len);
        if status != 0 || data_ptr.is_null() {
            return None;
        }
    }
    let slice = unsafe { std::slice::from_raw_parts(data_ptr as *const u8, byte_len) };
    Some(slice.to_vec())
}

fn read_napi_bool(env: NapiEnv, val: NapiValue) -> bool {
    let mut result: bool = false;
    unsafe { napi_get_value_bool(env, val, &mut result); }
    result
}

fn read_napi_u32(env: NapiEnv, val: NapiValue) -> u32 {
    let mut result: u32 = 0;
    unsafe { napi_get_value_uint32(env, val, &mut result); }
    result
}

fn return_arraybuffer(env: NapiEnv, data: &[u8]) -> NapiValue {
    let mut result: NapiValue = ptr::null_mut();
    unsafe {
        let mut buf_ptr: *mut c_void = ptr::null_mut();
        let status = napi_create_arraybuffer(env, data.len(), &mut buf_ptr, &mut result);
        if status == 0 && !buf_ptr.is_null() && !data.is_empty() {
            ptr::copy_nonoverlapping(data.as_ptr() as *const c_void, buf_ptr, data.len());
        }
    }
    result
}

fn json_to_napi(env: NapiEnv, value: &serde_json::Value) -> NapiValue {
    match value {
        serde_json::Value::Null => return_int32(env, 0),
        serde_json::Value::Bool(b) => {
            let mut result: NapiValue = ptr::null_mut();
            unsafe { napi_get_boolean(env, *b, &mut result); }
            result
        }
        serde_json::Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                return_int32(env, i as c_int)
            } else {
                return_int32(env, 0)
            }
        }
        serde_json::Value::String(s) => return_string(env, s),
        serde_json::Value::Array(arr) => {
            let mut js_arr: NapiValue = ptr::null_mut();
            unsafe { napi_create_array(env, &mut js_arr); }
            for (i, item) in arr.iter().enumerate() {
                let val = json_to_napi(env, item);
                unsafe { napi_set_element(env, js_arr, i as u32, val); }
            }
            js_arr
        }
        serde_json::Value::Object(map) => {
            let mut js_obj: NapiValue = ptr::null_mut();
            unsafe { napi_create_object(env, &mut js_obj); }
            for (key, val) in map {
                let napi_val = json_to_napi(env, val);
                let key_cstr = format!("{}\0", key);
                unsafe {
                    napi_set_named_property(env, js_obj, key_cstr.as_ptr() as *const c_char, napi_val);
                }
            }
            js_obj
        }
    }
}

fn return_json_object(env: NapiEnv, json_str: &str) -> NapiValue {
    match serde_json::from_str(json_str) {
        Ok(value) => json_to_napi(env, &value),
        Err(_) => return_string(env, json_str),
    }
}

// ── Helper macro to build NapiPropertyDescriptor ───────────────────
macro_rules! prop_desc {
    ($name:expr, $method:expr) => {
        NapiPropertyDescriptor {
            utf8name: concat!($name, "\0").as_ptr() as *const c_char,
            name: ptr::null_mut(),
            method: Some($method),
            getter: None,
            setter: None,
            value: ptr::null_mut(),
            attributes: 0,
            data: ptr::null_mut(),
        }
    };
}

// ── TSFN global handles ─────────────────────────────────────────────
// napi_tsfn_nonblocking = 0, napi_tsfn_release = 0 (same values)

static TSFN_STATE: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static TSFN_DATA: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static TSFN_LOG: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());
static TSFN_CONNECTOR: AtomicPtr<c_void> = AtomicPtr::new(ptr::null_mut());

// ── TSFN call_js callbacks ──────────────────────────────────────────

/// Generic call_js for string-based callbacks (state, log, connector state).
/// `context` points to a heap-allocated String that we free after calling JS.
extern "C" fn call_js_string(
    env: NapiEnv,
    _js_cb: NapiValue,
    _context: *mut c_void,
    data: *mut c_void,
) {
    if env.is_null() || data.is_null() { return; }
    let msg = unsafe { Box::from_raw(data as *mut String) };
    let mut js_str: NapiValue = ptr::null_mut();
    unsafe {
        napi_create_string_utf8(env, msg.as_ptr() as *const c_char, msg.len(), &mut js_str);
        let mut undefined: NapiValue = ptr::null_mut();
        napi_get_undefined(env, &mut undefined);
        napi_call_function(env, undefined, _js_cb, 1, &js_str, ptr::null_mut());
    }
}

/// call_js for binary data callbacks (onDataReceived).
/// `data` points to a heap-allocated Vec<u8>.
extern "C" fn call_js_data(
    env: NapiEnv,
    _js_cb: NapiValue,
    _context: *mut c_void,
    data: *mut c_void,
) {
    if env.is_null() || data.is_null() { return; }
    let bytes = unsafe { Box::from_raw(data as *mut Vec<u8>) };
    let mut js_ab: NapiValue = ptr::null_mut();
    unsafe {
        // Create ArrayBuffer by copying data into engine-managed memory.
        // We can't use napi_create_external_arraybuffer safely across threads
        // because the Box would need to outlive the call. Copy approach is simpler.
        let mut buf_ptr: *mut c_void = ptr::null_mut();
        let status = napi_create_arraybuffer(env, bytes.len(), &mut buf_ptr, &mut js_ab);
        if status == 0 && !buf_ptr.is_null() {
            ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_void, buf_ptr, bytes.len());
        }
        let mut undefined: NapiValue = ptr::null_mut();
        napi_get_undefined(env, &mut undefined);
        napi_call_function(env, undefined, _js_cb, 1, &js_ab, ptr::null_mut());
    }
}

// ── Public fire_* functions — called from background threads ────────

/// Fire a string callback to ArkTS (state change, log, connector state).
fn fire_tsf(tsf: &AtomicPtr<c_void>, msg: String) {
    let func = tsf.load(Ordering::Acquire);
    if func.is_null() { return; }
    let boxed = Box::new(msg);
    unsafe {
        napi_call_threadsafe_function(func, Box::into_raw(boxed) as *mut c_void, 0);
    }
}

pub fn fire_state(state: &str) {
    fire_tsf(&TSFN_STATE, state.to_string());
    crate::c_export::fire_c_state(state);
}

pub fn fire_log(msg: &str) {
    fire_tsf(&TSFN_LOG, msg.to_string());
    crate::c_export::fire_c_log(msg);
}

pub fn fire_connector_state(connected: bool) {
    fire_tsf(&TSFN_CONNECTOR, if connected { "true" } else { "false" }.to_string());
}

pub fn fire_data(data: &[u8]) {
    crate::c_export::fire_c_data(data);
    let func = TSFN_DATA.load(Ordering::Acquire);
    if func.is_null() { return; }
    let boxed = Box::new(data.to_vec());
    unsafe {
        napi_call_threadsafe_function(func, Box::into_raw(boxed) as *mut c_void, 0);
    }
}

// ── TSFN registration helpers ───────────────────────────────────────

unsafe fn register_tsf(
    env: NapiEnv,
    js_cb: NapiValue,
    name: &[u8],
    call_js: extern "C" fn(NapiEnv, NapiValue, *mut c_void, *mut c_void),
    target: &AtomicPtr<c_void>,
) -> c_int {
    // Release old TSFN handle if already registered
    let old = target.swap(ptr::null_mut(), Ordering::AcqRel);
    if !old.is_null() {
        napi_release_threadsafe_function(old, 0);
    }

    let mut async_name: NapiValue = ptr::null_mut();
    napi_create_string_utf8(env, name.as_ptr() as *const c_char, name.len() - 1, &mut async_name);
    let mut tsfn: NapiThreadsafeFunction = ptr::null_mut();
    let status = napi_create_threadsafe_function(
        env, js_cb, ptr::null_mut(), async_name,
        0, 1, // max_queue_size=0 (unlimited), initial_thread_count=1
        ptr::null_mut(), None, ptr::null_mut(),
        Some(call_js), &mut tsfn,
    );
    if status != 0 {
        hilog::log_error(&format!("register_tsf: create failed status={}", status));
        return -1;
    }
    target.store(tsfn, Ordering::Release);
    0
}

/// Release all TSFN handles (called from close).
pub fn release_all_tsfn() {
    for tsf in &[&TSFN_STATE, &TSFN_DATA, &TSFN_LOG, &TSFN_CONNECTOR] {
        let old = tsf.swap(ptr::null_mut(), Ordering::AcqRel);
        if !old.is_null() {
            unsafe { napi_release_threadsafe_function(old, 0); }
        }
    }
}

// ── NAPI callbacks ─────────────────────────────────────────────────

extern "C" fn init(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        return return_int32(env, -1);
    }
    let config_json = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let code = crate::client_napi::init(&config_json);
    return_int32(env, code)
}

extern "C" fn generate_token(_env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    let token = crate::client_napi::generate_token();
    return_string(_env, &token)
}

extern "C" fn gather_candidates(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        return return_json_object(env, "{\"error\":\"missing p2p_token\"}");
    }
    let p2p_token = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid token\"}"),
    };
    let result = crate::client_napi::gather_candidates(&p2p_token);
    return_json_object(env, &result)
}

extern "C" fn connect_connector(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 3) };
    if argc < 3 {
        return return_int32(env, -1);
    }
    let url = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let identifier = match read_napi_string(env, args[1]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let auth_token = match read_napi_string(env, args[2]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let code = crate::client_napi::connect_connector(&url, &identifier, &auth_token);
    return_int32(env, code)
}

extern "C" fn disconnect_connector(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    let code = crate::client_napi::disconnect_connector();
    return_int32(env, code)
}

extern "C" fn is_connector_registered(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    let v = crate::client_napi::is_connector_registered();
    return_int32(env, v)
}

extern "C" fn initiate_ice(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        return return_int32(env, -1);
    }
    let target_id = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let code = crate::client_napi::initiate_ice(&target_id);
    return_int32(env, code)
}

extern "C" fn stop_ice(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    let code = crate::client_napi::stop_ice();
    return_int32(env, code)
}

extern "C" fn send(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        return return_int32(env, -1);
    }
    // Try ArrayBuffer first, fall back to string
    if let Some(bytes) = read_napi_arraybuffer(env, args[0]) {
        let code = crate::client_napi::send(&bytes);
        return return_int32(env, code);
    }
    if let Some(text) = read_napi_string(env, args[0]) {
        let code = crate::client_napi::send_text(&text);
        return return_int32(env, code);
    }
    return_int32(env, -1)
}

extern "C" fn register_ids(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 4) };
    if argc < 4 {
        return return_json_object(env, "{\"error\":\"missing args\"}");
    }
    let app_id = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid appId\"}"),
    };
    let user_id = match read_napi_string(env, args[1]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid userId\"}"),
    };
    let odid = match read_napi_string(env, args[2]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid odid\"}"),
    };
    let push_token = match read_napi_string(env, args[3]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid pushToken\"}"),
    };
    let result = crate::client_napi::register_ids(&app_id, &user_id, &odid, &push_token);
    return_json_object(env, &result)
}

extern "C" fn query_ids(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 2) };
    if argc < 2 {
        return return_json_object(env, "{\"error\":\"missing args\"}");
    }
    let app_id = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid appId\"}"),
    };
    let user_id = match read_napi_string(env, args[1]) {
        Some(s) => s,
        None => return return_json_object(env, "{\"error\":\"invalid userId\"}"),
    };
    let result = crate::client_napi::query_ids(&app_id, &user_id);
    return_json_object(env, &result)
}

extern "C" fn ice_sdp_negotiate(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 3) };
    if argc < 2 {
        return return_int32(env, -1);
    }
    let peer_id = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let odid = match read_napi_string(env, args[1]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let is_device = argc >= 3 && read_napi_bool(env, args[2]);
    let code = crate::client_napi::ice_sdp_negotiate(&peer_id, &odid, is_device);
    return_int32(env, code)
}

extern "C" fn connect(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 4) };
    if argc < 2 {
        return return_int32(env, -1);
    }
    let peer_id = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let odid = match read_napi_string(env, args[1]) {
        Some(s) => s,
        None => return return_int32(env, -1),
    };
    let is_device = argc >= 3 && read_napi_bool(env, args[2]);
    let heartbeat_interval = if argc >= 4 { read_napi_u32(env, args[3]) } else { 30 };
    let code = crate::client_napi::connect(&peer_id, &odid, is_device, heartbeat_interval);
    return_int32(env, code)
}

extern "C" fn close(_env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    let code = crate::client_napi::close();
    return_int32(_env, code)
}

// ── TSFN callback registration functions ────────────────────────────

extern "C" fn on_state_change(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 { return return_int32(env, -1); }
    let code = unsafe { register_tsf(env, args[0], b"onStateChange\0", call_js_string, &TSFN_STATE) };
    return_int32(env, code)
}

extern "C" fn on_data_received(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 { return return_int32(env, -1); }
    let code = unsafe { register_tsf(env, args[0], b"onDataReceived\0", call_js_data, &TSFN_DATA) };
    return_int32(env, code)
}

extern "C" fn on_log(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 { return return_int32(env, -1); }
    let code = unsafe { register_tsf(env, args[0], b"onLog\0", call_js_string, &TSFN_LOG) };
    return_int32(env, code)
}

extern "C" fn on_connector_state_change(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 { return return_int32(env, -1); }
    let code = unsafe { register_tsf(env, args[0], b"onConnectorStateChange\0", call_js_string, &TSFN_CONNECTOR) };
    return_int32(env, code)
}

// ── Frame encode/decode exports ────────────────────────────────────

extern "C" fn encode_data_frame(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        return return_arraybuffer(env, &[]);
    }
    let text = match read_napi_string(env, args[0]) {
        Some(s) => s,
        None => return return_arraybuffer(env, &[]),
    };
    let frame = p2p_core::frame::encode_data_frame(&text);
    return_arraybuffer(env, &frame)
}

extern "C" fn encode_heartbeat_reply(env: NapiEnv, _info: NapiCallbackInfo) -> NapiValue {
    let frame = p2p_core::frame::encode_heartbeat_reply();
    return_arraybuffer(env, &frame)
}

extern "C" fn parse_frame(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        let mut js_obj: NapiValue = ptr::null_mut();
        unsafe {
            napi_create_object(env, &mut js_obj);
            let type_val = { let mut v: NapiValue = ptr::null_mut(); napi_create_uint32(env, 0, &mut v); v };
            napi_set_named_property(env, js_obj, b"type\0".as_ptr() as *const c_char, type_val);
            let payload_val = return_arraybuffer(env, &[]);
            napi_set_named_property(env, js_obj, b"payload\0".as_ptr() as *const c_char, payload_val);
        }
        return js_obj;
    }
    let data = match read_napi_arraybuffer(env, args[0]) {
        Some(b) => b,
        None => {
            let mut js_obj: NapiValue = ptr::null_mut();
            unsafe {
                napi_create_object(env, &mut js_obj);
                let type_val = { let mut v: NapiValue = ptr::null_mut(); napi_create_uint32(env, 0, &mut v); v };
                napi_set_named_property(env, js_obj, b"type\0".as_ptr() as *const c_char, type_val);
                let payload_val = return_arraybuffer(env, &[]);
                napi_set_named_property(env, js_obj, b"payload\0".as_ptr() as *const c_char, payload_val);
            }
            return js_obj;
        }
    };
    match p2p_core::frame::parse_frame(&data) {
        Some(parsed) => {
            let mut js_obj: NapiValue = ptr::null_mut();
            unsafe {
                napi_create_object(env, &mut js_obj);
                let type_val = {
                    let mut v: NapiValue = ptr::null_mut();
                    napi_create_uint32(env, parsed.frame_type, &mut v);
                    v
                };
                napi_set_named_property(env, js_obj, b"type\0".as_ptr() as *const c_char, type_val);
                let payload_val = return_arraybuffer(env, &parsed.payload);
                napi_set_named_property(env, js_obj, b"payload\0".as_ptr() as *const c_char, payload_val);
            }
            js_obj
        }
        None => {
            let mut js_obj: NapiValue = ptr::null_mut();
            unsafe {
                napi_create_object(env, &mut js_obj);
                let type_val = {
                    let mut v: NapiValue = ptr::null_mut();
                    napi_create_uint32(env, 0, &mut v);
                    v
                };
                napi_set_named_property(env, js_obj, b"type\0".as_ptr() as *const c_char, type_val);
                let payload_val = return_arraybuffer(env, &[]);
                napi_set_named_property(env, js_obj, b"payload\0".as_ptr() as *const c_char, payload_val);
            }
            js_obj
        }
    }
}

extern "C" fn is_stun_message(env: NapiEnv, info: NapiCallbackInfo) -> NapiValue {
    let (argc, args) = unsafe { get_cb_args(env, info, 1) };
    if argc < 1 {
        let mut result: NapiValue = ptr::null_mut();
        unsafe { napi_get_boolean(env, false, &mut result); }
        return result;
    }
    let data = match read_napi_arraybuffer(env, args[0]) {
        Some(b) => b,
        None => {
            let mut result: NapiValue = ptr::null_mut();
            unsafe { napi_get_boolean(env, false, &mut result); }
            return result;
        }
    };
    let is_stun = p2p_core::frame::is_stun_message(&data);
    let mut result: NapiValue = ptr::null_mut();
    unsafe { napi_get_boolean(env, is_stun, &mut result); }
    result
}

// ── Module registration ────────────────────────────────────────────

extern "C" fn init_module(env: NapiEnv, exports: NapiValue) -> NapiValue {
    hilog::log_info("INIT_MODULE: called");

    let descriptors: [NapiPropertyDescriptor; 22] = [
        // External interfaces
        prop_desc!("init", init),
        prop_desc!("registerIds", register_ids),
        prop_desc!("queryIds", query_ids),
        prop_desc!("connect", connect),
        prop_desc!("onStateChange", on_state_change),
        prop_desc!("send", send),
        prop_desc!("onDataReceived", on_data_received),
        prop_desc!("close", close),
        // Internal interfaces
        prop_desc!("generateToken", generate_token),
        prop_desc!("gatherCandidates", gather_candidates),
        prop_desc!("iceSdpNegotiate", ice_sdp_negotiate),
        prop_desc!("encodeDataFrame", encode_data_frame),
        prop_desc!("encodeHeartbeatReply", encode_heartbeat_reply),
        prop_desc!("parseFrame", parse_frame),
        prop_desc!("isStunMessage", is_stun_message),
        prop_desc!("onLog", on_log),
        prop_desc!("onConnectorStateChange", on_connector_state_change),
        prop_desc!("connectConnector", connect_connector),
        prop_desc!("disconnectConnector", disconnect_connector),
        prop_desc!("isConnectorRegistered", is_connector_registered),
        prop_desc!("initiateIce", initiate_ice),
        prop_desc!("stopIce", stop_ice),
    ];

    let status = unsafe {
        napi_define_properties(env, exports, descriptors.len(), descriptors.as_ptr())
    };
    hilog::log_info(&format!("INIT_MODULE: napi_define_properties status={}", status));
    exports
}

const MODULE_NAME: &[u8] = b"libppsdk.so\0";

static MODULE: NapiModule = NapiModule {
    nm_version: 1,
    nm_flags: 0,
    nm_filename: ptr::null(),
    nm_register_func: Some(init_module),
    nm_modname: MODULE_NAME.as_ptr() as *const c_char,
    nm_priv: ptr::null_mut(),
    reserved: [ptr::null_mut(); 4],
};

extern "C" fn napi_ctor() {
    hilog::log_info("NAPI_CTOR: registering module");
    unsafe { napi_module_register(&MODULE); }
    hilog::log_info("NAPI_CTOR: module registered");
}

#[used]
#[link_section = ".init_array"]
static NAPI_CTOR: extern "C" fn() = napi_ctor;
