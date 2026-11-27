// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashSet;
use std::env;
#[cfg(not(unix))]
use std::sync::atomic::AtomicU32;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use lexe_utils::sysinfo::ARCH;
use rquickjs::{
    convert::Coerced, function::Constructor, prelude::Opt, qjs, Array, Ctx, Exception, Function,
    Object, Result, Value,
};

static PROCESS_EXEC_ARGV: OnceLock<Vec<String>> = OnceLock::new();
static SOURCE_MAPS_DEFAULT: AtomicBool = AtomicBool::new(false);
static PROCESS_TITLE: OnceLock<Mutex<String>> = OnceLock::new();
#[cfg(not(unix))]
static FALLBACK_UMASK: AtomicU32 = AtomicU32::new(0o022);

pub fn set_exec_argv(argv: Vec<String>) {
    let _ = PROCESS_EXEC_ARGV.set(argv);
}

pub fn exec_argv() -> Vec<String> {
    PROCESS_EXEC_ARGV.get().cloned().unwrap_or_default()
}

pub fn set_source_maps_default(enabled: bool) {
    SOURCE_MAPS_DEFAULT.store(enabled, Ordering::Relaxed);
}

pub fn source_maps_default() -> bool {
    SOURCE_MAPS_DEFAULT.load(Ordering::Relaxed)
}

pub fn init_title(initial: String) {
    let cell = PROCESS_TITLE.get_or_init(|| Mutex::new(initial.clone()));
    if let Ok(mut g) = cell.lock() {
        if g.is_empty() {
            *g = initial;
        }
    }
}

pub fn get_title() -> String {
    PROCESS_TITLE
        .get()
        .and_then(|m| m.lock().ok().map(|s| s.clone()))
        .unwrap_or_else(|| "lexe".to_string())
}

pub fn set_title(new_title: String) {
    let cell = PROCESS_TITLE.get_or_init(|| Mutex::new(String::new()));
    if let Ok(mut g) = cell.lock() {
        *g = new_title.clone();
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(c) = std::ffi::CString::new(new_title.chars().take(15).collect::<String>()) {
            unsafe {
                libc::prctl(libc::PR_SET_NAME, c.as_ptr(), 0, 0, 0);
            }
        }
    }
}

pub fn parent_pid() -> u32 {
    #[cfg(unix)]
    {
        unsafe { libc::getppid() as u32 }
    }
    #[cfg(not(unix))]
    {
        0
    }
}

pub fn exec_path(argv0: &str) -> String {
    #[cfg(not(target_family = "wasm"))]
    if let Ok(path) = env::current_exe() {
        return path.to_string_lossy().into_owned();
    }
    argv0.to_string()
}

pub fn throw_coded<'js>(ctx: &Ctx<'js>, is_type: bool, code: &str, msg: &str) -> rquickjs::Error {
    let ctor_name = if is_type { "TypeError" } else { "RangeError" };
    if let Ok(ctor) = ctx.globals().get::<_, Constructor>(ctor_name) {
        if let Ok(obj) = ctor.construct::<_, Object>((msg,)) {
            let _ = obj.set("code", code);
            return ctx.throw(obj.into_value());
        }
    }
    Exception::throw_message(ctx, msg)
}

fn quickjs_mem_usage(ctx: &Ctx<'_>) -> qjs::JSMemoryUsage {
    unsafe {
        let mut usage = std::mem::zeroed();
        let rt = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
        qjs::JS_ComputeMemoryUsage(rt, &mut usage);
        usage
    }
}

#[allow(deprecated)]
fn os_rss_bytes(fallback: f64) -> f64 {
    #[cfg(target_os = "macos")]
    unsafe {
        let mut info: libc::mach_task_basic_info = std::mem::zeroed();
        let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
        let rc = libc::task_info(
            libc::mach_task_self_,
            libc::MACH_TASK_BASIC_INFO,
            (&raw mut info) as libc::task_info_t,
            &raw mut count,
        );
        if rc == libc::KERN_SUCCESS {
            return info.resident_size as f64;
        }
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &raw mut ru) == 0 && ru.ru_maxrss > 0 {
            return ru.ru_maxrss as f64;
        }
        fallback
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            if let Some(pages_str) = statm.split_whitespace().nth(1) {
                if let Ok(pages) = pages_str.parse::<u64>() {
                    let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) } as u64;
                    return (pages.saturating_mul(page_size.max(4096))) as f64;
                }
            }
        }
        unsafe {
            let mut ru: libc::rusage = std::mem::zeroed();
            if libc::getrusage(libc::RUSAGE_SELF, &raw mut ru) == 0 && ru.ru_maxrss > 0 {
                return (ru.ru_maxrss as f64) * 1024.0;
            }
        }
        fallback
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        fallback
    }
}

pub fn memory_usage<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let usage = quickjs_mem_usage(&ctx);
    let heap_total = usage.malloc_size.max(0) as f64;
    let heap_used = usage.memory_used_size.max(0) as f64;
    let external = usage.binary_object_size.max(0) as f64;
    let rss = os_rss_bytes(heap_total + external);

    let obj = Object::new(ctx)?;
    obj.set("rss", rss)?;
    obj.set("heapTotal", heap_total)?;
    obj.set("heapUsed", heap_used)?;
    obj.set("external", external)?;
    obj.set("arrayBuffers", external)?;
    Ok(obj)
}

pub fn memory_usage_rss(ctx: Ctx<'_>) -> f64 {
    let usage = quickjs_mem_usage(&ctx);
    let fallback = usage.malloc_size.max(0) as f64 + usage.binary_object_size.max(0) as f64;
    os_rss_bytes(fallback)
}

#[cfg(unix)]
fn raw_cpu_micros() -> (f64, f64) {
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &raw mut ru) == 0 {
            let user = (ru.ru_utime.tv_sec as f64) * 1_000_000.0 + (ru.ru_utime.tv_usec as f64);
            let system = (ru.ru_stime.tv_sec as f64) * 1_000_000.0 + (ru.ru_stime.tv_usec as f64);
            return (user, system);
        }
    }
    (0.0, 0.0)
}

#[cfg(not(unix))]
fn raw_cpu_micros() -> (f64, f64) {
    (0.0, 0.0)
}

pub fn cpu_usage<'js>(ctx: Ctx<'js>, prev: Opt<Value<'js>>) -> Result<Object<'js>> {
    build_cpu_usage(&ctx, raw_cpu_micros(), prev)
}

pub fn thread_cpu_usage<'js>(ctx: Ctx<'js>, prev: Opt<Value<'js>>) -> Result<Object<'js>> {
    build_cpu_usage(&ctx, raw_thread_cpu_micros(), prev)
}

#[cfg(target_os = "linux")]
fn raw_thread_cpu_micros() -> (f64, f64) {
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_THREAD, &raw mut ru) == 0 {
            let user = (ru.ru_utime.tv_sec as f64) * 1_000_000.0 + (ru.ru_utime.tv_usec as f64);
            let system = (ru.ru_stime.tv_sec as f64) * 1_000_000.0 + (ru.ru_stime.tv_usec as f64);
            return (user, system);
        }
    }
    raw_cpu_micros()
}

#[cfg(not(target_os = "linux"))]
fn raw_thread_cpu_micros() -> (f64, f64) {
    raw_cpu_micros()
}

fn build_cpu_usage<'js>(
    ctx: &Ctx<'js>,
    (mut user, mut system): (f64, f64),
    prev: Opt<Value<'js>>,
) -> Result<Object<'js>> {
    if let Some(prev_val) = prev.0 {
        if !prev_val.is_undefined() {
            let Some(prev_obj) = prev_val.as_object() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"previousValue\" argument must be of type object",
                ));
            };
            let prev_user: Value = prev_obj.get("user")?;
            let Some(pu) = prev_user.as_number() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"previousValue.user\" property must be of type number",
                ));
            };
            if !pu.is_finite() || pu < 0.0 {
                return Err(throw_coded(
                    ctx,
                    false,
                    "ERR_INVALID_ARG_VALUE",
                    "The value of \"previousValue.user\" is out of range",
                ));
            }
            let prev_sys: Value = prev_obj.get("system")?;
            let Some(ps) = prev_sys.as_number() else {
                return Err(throw_coded(
                    ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"previousValue.system\" property must be of type number",
                ));
            };
            if !ps.is_finite() || ps < 0.0 {
                return Err(throw_coded(
                    ctx,
                    false,
                    "ERR_INVALID_ARG_VALUE",
                    "The value of \"previousValue.system\" is out of range",
                ));
            }
            user = (user - pu).max(0.0);
            system = (system - ps).max(0.0);
        }
    }

    let obj = Object::new(ctx.clone())?;
    obj.set("user", user as u64 as f64)?;
    obj.set("system", system as u64 as f64)?;
    Ok(obj)
}

pub fn resource_usage<'js>(ctx: Ctx<'js>) -> Result<Object<'js>> {
    let obj = Object::new(ctx)?;
    #[cfg(unix)]
    unsafe {
        let mut ru: libc::rusage = std::mem::zeroed();
        if libc::getrusage(libc::RUSAGE_SELF, &raw mut ru) == 0 {
            let user_us = (ru.ru_utime.tv_sec as f64) * 1_000_000.0 + (ru.ru_utime.tv_usec as f64);
            let sys_us = (ru.ru_stime.tv_sec as f64) * 1_000_000.0 + (ru.ru_stime.tv_usec as f64);
            #[cfg(target_os = "macos")]
            let max_rss_kb = (ru.ru_maxrss as f64) / 1024.0;
            #[cfg(not(target_os = "macos"))]
            let max_rss_kb = ru.ru_maxrss as f64;

            obj.set("userCPUTime", user_us as u64 as f64)?;
            obj.set("systemCPUTime", sys_us as u64 as f64)?;
            obj.set("maxRSS", max_rss_kb as u64 as f64)?;
            obj.set("sharedMemorySize", ru.ru_ixrss as f64)?;
            obj.set("unsharedDataSize", ru.ru_idrss as f64)?;
            obj.set("unsharedStackSize", ru.ru_isrss as f64)?;
            obj.set("minorPageFault", ru.ru_minflt as f64)?;
            obj.set("majorPageFault", ru.ru_majflt as f64)?;
            obj.set("swappedOut", ru.ru_nswap as f64)?;
            obj.set("fsRead", ru.ru_inblock as f64)?;
            obj.set("fsWrite", ru.ru_oublock as f64)?;
            obj.set("ipcSent", ru.ru_msgsnd as f64)?;
            obj.set("ipcReceived", ru.ru_msgrcv as f64)?;
            obj.set("signalsCount", ru.ru_nsignals as f64)?;
            obj.set("voluntaryContextSwitches", ru.ru_nvcsw as f64)?;
            obj.set("involuntaryContextSwitches", ru.ru_nivcsw as f64)?;
            return Ok(obj);
        }
    }

    for k in [
        "userCPUTime",
        "systemCPUTime",
        "maxRSS",
        "sharedMemorySize",
        "unsharedDataSize",
        "unsharedStackSize",
        "minorPageFault",
        "majorPageFault",
        "swappedOut",
        "fsRead",
        "fsWrite",
        "ipcSent",
        "ipcReceived",
        "signalsCount",
        "voluntaryContextSwitches",
        "involuntaryContextSwitches",
    ] {
        obj.set(k, 0)?;
    }
    Ok(obj)
}

pub fn constrained_memory() -> f64 {
    #[cfg(target_os = "linux")]
    {
        if let Ok(val) = std::fs::read_to_string("/sys/fs/cgroup/memory.max") {
            let trimmed = val.trim();
            if trimmed != "max" {
                if let Ok(n) = trimmed.parse::<u64>() {
                    return n as f64;
                }
            }
        }
        if let Ok(val) = std::fs::read_to_string("/sys/fs/cgroup/memory/memory.limit_in_bytes") {
            if let Ok(n) = val.trim().parse::<u64>() {
                // Linux cgroup v1 reports ~9e18 when unconstrained.
                if n < (1u64 << 60) {
                    return n as f64;
                }
            }
        }
    }
    0.0
}

#[allow(deprecated)]
pub fn available_memory() -> f64 {
    let limit = constrained_memory();
    let host_avail: f64 = {
        #[cfg(target_os = "macos")]
        unsafe {
            let mut stats: libc::vm_statistics64 = std::mem::zeroed();
            let mut count = libc::HOST_VM_INFO64_COUNT;
            let host = libc::mach_host_self();
            let rc = libc::host_statistics64(
                host,
                libc::HOST_VM_INFO64,
                (&raw mut stats) as libc::host_info64_t,
                &raw mut count,
            );
            let page_size = libc::sysconf(libc::_SC_PAGESIZE).max(4096) as u64;
            if rc == libc::KERN_SUCCESS {
                let pages = (stats.free_count as u64).saturating_add(stats.inactive_count as u64);
                (pages.saturating_mul(page_size)) as f64
            } else {
                let total_pages = libc::sysconf(libc::_SC_PHYS_PAGES).max(0) as u64;
                (total_pages.saturating_mul(page_size)) as f64
            }
        }
        #[cfg(target_os = "linux")]
        {
            if let Ok(meminfo) = std::fs::read_to_string("/proc/meminfo") {
                for line in meminfo.lines() {
                    if let Some(rest) = line.strip_prefix("MemAvailable:") {
                        if let Some(kb_str) = rest.split_whitespace().next() {
                            if let Ok(kb) = kb_str.parse::<u64>() {
                                return if limit > 0.0 {
                                    (kb as f64 * 1024.0).min(limit)
                                } else {
                                    kb as f64 * 1024.0
                                };
                            }
                        }
                    }
                }
            }
            0.0
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            0.0
        }
    };

    if limit > 0.0 && host_avail > 0.0 {
        host_avail.min(limit)
    } else if limit > 0.0 {
        limit
    } else {
        host_avail
    }
}

pub fn umask<'js>(ctx: Ctx<'js>, mask: Opt<Value<'js>>) -> Result<u32> {
    let parsed: Option<u32> = match mask.0 {
        None => None,
        Some(v) if v.is_undefined() => None,
        Some(v) => {
            if let Some(n) = v.as_number() {
                if !n.is_finite() || n.fract() != 0.0 || n < 0.0 || n > (u32::MAX as f64) {
                    return Err(throw_coded(
                        &ctx,
                        false,
                        "ERR_OUT_OF_RANGE",
                        "The value of \"mask\" is out of range",
                    ));
                }
                Some(n as u32)
            } else if let Some(s) = v.as_string() {
                let raw = s.to_string()?;
                let trimmed = raw.trim();
                let stripped = trimmed
                    .strip_prefix("0o")
                    .or_else(|| trimmed.strip_prefix("0O"))
                    .unwrap_or(trimmed);
                let val = u32::from_str_radix(stripped, 8).map_err(|_| {
                    throw_coded(
                        &ctx,
                        true,
                        "ERR_INVALID_ARG_VALUE",
                        &format!("Invalid octal umask string: {raw}"),
                    )
                })?;
                Some(val)
            } else {
                return Err(throw_coded(
                    &ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"mask\" argument must be a number or octal string",
                ));
            }
        },
    };

    #[cfg(unix)]
    {
        match parsed {
            None => unsafe {
                let prev = libc::umask(0);
                libc::umask(prev);
                Ok(prev as u32)
            },
            Some(new_mask) => unsafe {
                let prev = libc::umask((new_mask & 0o777) as libc::mode_t);
                Ok(prev as u32)
            },
        }
    }
    #[cfg(not(unix))]
    {
        match parsed {
            None => Ok(FALLBACK_UMASK.load(Ordering::Relaxed)),
            Some(new_mask) => Ok(FALLBACK_UMASK.swap(new_mask & 0o777, Ordering::Relaxed)),
        }
    }
}

pub fn get_active_resources_info<'js>(ctx: Ctx<'js>) -> Result<Array<'js>> {
    let rt_ptr = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    let rt_key = lexe_utils::event_loop::rt_key_from_ptr(rt_ptr as *mut std::ffi::c_void);
    let names = lexe_utils::event_loop::active_resource_names(rt_key);
    let arr = Array::new(ctx)?;
    for (i, name) in names.into_iter().enumerate() {
        arr.set(i, name)?;
    }
    Ok(arr)
}

pub fn build_config_obj<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let config = Object::new(ctx.clone())?;
    let target_defaults = Object::new(ctx.clone())?;
    let variables = Object::new(ctx.clone())?;
    variables.set("host_arch", ARCH)?;
    variables.set("target_arch", ARCH)?;
    variables.set("node_shared", false)?;
    config.set("target_defaults", target_defaults)?;
    config.set("variables", variables)?;
    Ok(config)
}

pub fn build_features_obj<'js>(ctx: &Ctx<'js>) -> Result<Object<'js>> {
    let features = Object::new(ctx.clone())?;
    features.set("inspector", false)?;
    features.set("debug", false)?;
    features.set("uv", false)?;
    features.set("ipv6", true)?;
    features.set("tls_alpn", true)?;
    features.set("tls_sni", true)?;
    features.set("tls_ocsp", false)?;
    features.set("tls", true)?;
    features.set("cached_builtins", true)?;
    features.set("require_module", true)?;
    features.set("typescript", "strip")?;
    Ok(features)
}

pub fn build_allowed_env_flags<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    let ctor: Constructor = ctx.globals().get("Set")?;
    let flags = [
        "--no-warnings",
        "--trace-warnings",
        "--no-experimental-sqlite",
        "--enable-source-maps",
        "--permission",
        "--allow-fs-read",
        "--allow-fs-write",
        "--allow-child-process",
        "--allow-worker",
        "--allow-addons",
        "--allow-wasi",
        "-e",
        "--eval",
        "--import",
        "-v",
        "--version",
        "-h",
        "--help",
    ];
    let arr = Array::new(ctx.clone())?;
    for (i, f) in flags.iter().enumerate() {
        arr.set(i, *f)?;
    }
    ctor.construct((arr,))
}

pub fn build_permission_obj<'js>(ctx: &Ctx<'js>) -> Result<Value<'js>> {
    if !lexe_utils::permissions::is_enabled() {
        return Ok(rquickjs::Undefined.into_value(ctx.clone()));
    }
    let perm = Object::new(ctx.clone())?;
    let has_fn = rquickjs::Function::new(
        ctx.clone(),
        |ctx: Ctx<'_>, scope: Value<'_>, reference: Opt<Value<'_>>| -> Result<bool> {
            let Some(scope_str) = scope.as_string() else {
                return Err(throw_coded(
                    &ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"scope\" argument must be of type string",
                ));
            };
            let scope = scope_str.to_string()?;
            let ref_opt = match reference.0 {
                None => None,
                Some(v) if v.is_undefined() => None,
                Some(v) => {
                    let Some(s) = v.as_string() else {
                        return Err(throw_coded(
                            &ctx,
                            true,
                            "ERR_INVALID_ARG_TYPE",
                            "The \"reference\" argument must be of type string",
                        ));
                    };
                    Some(s.to_string()?)
                },
            };
            Ok(lexe_utils::permissions::has(&scope, ref_opt.as_deref()))
        },
    )?;
    perm.set("has", has_fn)?;
    Ok(perm.into_value())
}

pub fn parse_env_content(content: &str) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let bytes = content.as_bytes();
    let len = bytes.len();
    let mut i = 0usize;

    while i < len {
        // Skip leading horizontal whitespace and blank lines.
        while i < len && matches!(bytes[i], b' ' | b'\t' | b'\r' | b'\n') {
            i += 1;
        }
        if i >= len {
            break;
        }
        if bytes[i] == b'#' {
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Parse up to `=` on the current line.
        let line_start = i;
        while i < len && !matches!(bytes[i], b'=' | b'\n' | b'\r') {
            i += 1;
        }
        if i >= len || bytes[i] != b'=' {
            // No `=` on this line, skip it.
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        let mut raw_key = content[line_start..i].trim();
        if let Some(after_export) = raw_key.strip_prefix("export") {
            if after_export.starts_with(char::is_whitespace) {
                raw_key = after_export.trim_start();
            }
        }
        i += 1; // skip `=`

        while i < len && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }

        let val = if i < len && matches!(bytes[i], b'"' | b'\'' | b'`') {
            let quote = bytes[i];
            i += 1;
            let mut out = String::new();
            while i < len {
                let b = bytes[i];
                if quote == b'"' && b == b'\\' && i + 1 < len {
                    let next = bytes[i + 1];
                    match next {
                        b'n' => {
                            out.push('\n');
                            i += 2;
                            continue;
                        },
                        b'r' => {
                            out.push('\r');
                            i += 2;
                            continue;
                        },
                        b't' => {
                            out.push('\t');
                            i += 2;
                            continue;
                        },
                        b'"' | b'\\' => {
                            out.push(next as char);
                            i += 2;
                            continue;
                        },
                        _ => {},
                    }
                }
                if b == quote {
                    i += 1;
                    // Skip rest of closing line.
                    while i < len && bytes[i] != b'\n' {
                        i += 1;
                    }
                    break;
                }
                let ch = content[i..].chars().next().unwrap();
                out.push(ch);
                i += ch.len_utf8();
            }
            out
        } else {
            let val_start = i;
            let mut val_end = len;
            while i < len && bytes[i] != b'\n' && bytes[i] != b'\r' {
                if bytes[i] == b'#' && (i == val_start || matches!(bytes[i - 1], b' ' | b'\t')) {
                    val_end = i;
                    while i < len && bytes[i] != b'\n' {
                        i += 1;
                    }
                    break;
                }
                i += 1;
                val_end = i;
            }
            content[val_start..val_end].trim().to_string()
        };

        if !raw_key.is_empty() {
            pairs.push((raw_key.to_string(), val));
        }
    }

    pairs
}

pub fn load_env_file<'js>(ctx: Ctx<'js>, path: Opt<Value<'js>>) -> Result<()> {
    let file_path = match path.0 {
        None => ".env".to_string(),
        Some(v) if v.is_undefined() => ".env".to_string(),
        Some(v) => {
            if let Some(s) = v.as_string() {
                let raw = s.to_string()?;
                raw.strip_prefix("file://").unwrap_or(&raw).to_string()
            } else if let Some(obj) = v.as_object() {
                if let Ok(href) = obj.get::<_, String>("href") {
                    if let Some(p) = href.strip_prefix("file://") {
                        p.to_string()
                    } else {
                        return Err(throw_coded(
                            &ctx,
                            true,
                            "ERR_INVALID_URL_SCHEME",
                            "The URL must be of scheme file",
                        ));
                    }
                } else {
                    let coerced: Coerced<String> = obj.clone().into_value().get()?;
                    coerced.0
                }
            } else {
                return Err(throw_coded(
                    &ctx,
                    true,
                    "ERR_INVALID_ARG_TYPE",
                    "The \"path\" argument must be of type string or an instance of Buffer or URL",
                ));
            }
        },
    };

    let content = std::fs::read_to_string(&file_path).map_err(|err| {
        let code = match err.kind() {
            std::io::ErrorKind::NotFound => "ENOENT",
            std::io::ErrorKind::PermissionDenied => "EACCES",
            _ => "ERR_SYSTEM_ERROR",
        };
        let errno = err.raw_os_error().map(|e| -e.abs()).unwrap_or(-2);
        let msg = format!("{code}: {err}, open '{file_path}'");
        if let Ok(ex) = Exception::from_message(ctx.clone(), &msg) {
            let o = ex.as_object();
            let _ = o.set("code", code);
            let _ = o.set("errno", errno);
            let _ = o.set("syscall", "open");
            let _ = o.set("path", file_path.as_str());
            return ctx.throw(ex.into());
        }
        Exception::throw_message(&ctx, &msg)
    })?;

    let process: Object = ctx.globals().get("process")?;
    let env_obj: Object = process.get("env")?;
    let has_own: Function = ctx.eval("(obj, k) => Object.prototype.hasOwnProperty.call(obj, k)")?;
    let mut seen: HashSet<String> = HashSet::new();
    for (k, v) in parse_env_content(&content) {
        if seen.contains(&k) {
            continue;
        }
        seen.insert(k.clone());
        let exists: bool = has_own.call((env_obj.clone(), k.as_str()))?;
        if !exists {
            env_obj.set(k.as_str(), v)?;
        }
    }
    Ok(())
}

pub fn write_report_file(ctx: &Ctx<'_>, path: &str, json: &str) -> Result<()> {
    std::fs::write(path, json).map_err(|err| {
        Exception::throw_message(ctx, &format!("Failed to write report to '{path}': {err}"))
    })
}
