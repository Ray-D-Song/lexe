// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::os::raw::{c_char, c_void};

pub type napi_env = *mut c_void;
pub type napi_value = *mut c_void;
pub type napi_ref = *mut c_void;
pub type napi_handle_scope = *mut c_void;
pub type napi_escapable_handle_scope = *mut c_void;
pub type napi_callback_info = *mut c_void;
pub type napi_deferred = *mut c_void;
pub type napi_async_work = *mut c_void;
pub type napi_threadsafe_function = *mut c_void;

pub type napi_callback =
    Option<unsafe extern "C" fn(env: napi_env, info: napi_callback_info) -> napi_value>;
pub type napi_finalize =
    Option<unsafe extern "C" fn(env: napi_env, data: *mut c_void, hint: *mut c_void)>;
pub type napi_async_execute_callback =
    Option<unsafe extern "C" fn(env: napi_env, data: *mut c_void)>;
pub type napi_async_complete_callback =
    Option<unsafe extern "C" fn(env: napi_env, status: napi_status, data: *mut c_void)>;
pub type napi_threadsafe_function_call_js = Option<
    unsafe extern "C" fn(
        env: napi_env,
        js_callback: napi_value,
        context: *mut c_void,
        data: *mut c_void,
    ),
>;
pub type napi_addon_register_func =
    unsafe extern "C" fn(env: napi_env, exports: napi_value) -> napi_value;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct napi_property_descriptor {
    pub utf8name: *const c_char,
    pub name: napi_value,
    pub method: napi_callback,
    pub getter: napi_callback,
    pub setter: napi_callback,
    pub value: napi_value,
    pub attributes: napi_property_attributes,
    pub data: *mut c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct napi_extended_error_info {
    pub error_message: *const c_char,
    pub engine_reserved: *mut c_void,
    pub error_code: napi_status,
    pub engine_error_code: u32,
}

/// Matches Node.js `napi_node_version` (node_api_types.h / js_native_api_types.h).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct napi_node_version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub release: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct napi_type_tag {
    pub lower: u64,
    pub upper: u64,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct napi_module {
    pub nm_version: i32,
    pub nm_flags: u32,
    pub nm_filename: *const c_char,
    pub nm_register_func: napi_addon_register_func,
    pub nm_modname: *const c_char,
    pub nm_priv: *mut c_void,
    pub reserved: [*mut c_void; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum napi_status {
    napi_ok = 0,
    napi_invalid_arg = 1,
    napi_object_expected = 2,
    napi_string_expected = 3,
    napi_name_expected = 4,
    napi_function_expected = 5,
    napi_number_expected = 6,
    napi_boolean_expected = 7,
    napi_array_expected = 8,
    napi_generic_failure = 9,
    napi_pending_exception = 10,
    napi_cancelled = 11,
    napi_escape_called_twice = 12,
    napi_handle_scope_mismatch = 13,
    napi_callback_scope_mismatch = 14,
    napi_queue_full = 15,
    napi_closing = 16,
    napi_bigint_expected = 17,
    napi_date_expected = 18,
    napi_arraybuffer_expected = 19,
    napi_detachable_arraybuffer_expected = 20,
    napi_would_deadlock = 21,
    napi_no_external_buffers_allowed = 22,
    napi_cannot_run_js = 23,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum napi_valuetype {
    napi_undefined = 0,
    napi_null = 1,
    napi_boolean = 2,
    napi_number = 3,
    napi_bigint = 4,
    napi_string = 5,
    napi_symbol = 6,
    napi_object = 7,
    napi_function = 8,
    napi_external = 9,
    napi_bigint_object = 10,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum napi_typedarray_type {
    napi_int8_array = 0,
    napi_uint8_array = 1,
    napi_uint8_clamped_array = 2,
    napi_int16_array = 3,
    napi_uint16_array = 4,
    napi_int32_array = 5,
    napi_uint32_array = 6,
    napi_float32_array = 7,
    napi_float64_array = 8,
    napi_bigint64_array = 9,
    napi_biguint64_array = 10,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum napi_property_attributes {
    napi_default = 0,
    napi_writable = 1 << 0,
    napi_enumerable = 1 << 1,
    napi_configurable = 1 << 2,
    napi_static = 1 << 10,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum napi_threadsafe_function_release_mode {
    napi_tsfn_release = 0,
    napi_tsfn_abort = 1,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum napi_threadsafe_function_call_mode {
    napi_tsfn_nonblocking = 0,
    napi_tsfn_blocking = 1,
}

pub const NAPI_AUTO_LENGTH: usize = usize::MAX;

pub const NODE_API_DEFAULT_MODULE_API_VERSION: i32 = 8;
