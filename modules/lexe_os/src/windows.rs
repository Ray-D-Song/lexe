// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use std::ffi::c_void;

use lexe_utils::result::ResultExt;
use once_cell::sync::Lazy;
use rquickjs::{
    prelude::{Opt, Rest},
    Ctx, Exception, IntoJs, Null, Object,
};

use windows_registry::LOCAL_MACHINE;
use windows_result::Result;
use windows_version::OsVersion;

use crate::get_home_dir;

type Handle = *mut c_void;

const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const PROCESS_SET_INFORMATION: u32 = 0x0200;

const IDLE_PRIORITY_CLASS: u32 = 0x0000_0040;
const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;
const NORMAL_PRIORITY_CLASS: u32 = 0x0000_0020;
const ABOVE_NORMAL_PRIORITY_CLASS: u32 = 0x0000_8000;
const HIGH_PRIORITY_CLASS: u32 = 0x0000_0080;
const REALTIME_PRIORITY_CLASS: u32 = 0x0000_0100;

extern "system" {
    fn GetCurrentProcess() -> Handle;
    fn OpenProcess(dw_desired_access: u32, b_inherit_handle: i32, dw_process_id: u32) -> Handle;
    fn GetPriorityClass(h_process: Handle) -> u32;
    fn SetPriorityClass(h_process: Handle, dw_priority_class: u32) -> i32;
    fn CloseHandle(h_object: Handle) -> i32;
}

static OS_RELEASE: Lazy<String> = Lazy::new(release);
static OS_VERSION: Lazy<String> = Lazy::new(|| version().unwrap_or_default());
pub static EOL: &str = "\r\n";
pub static DEV_NULL: &str = "\\.\nul";

fn open_priority_handle(pid: u32, access: u32) -> Option<(Handle, bool)> {
    if pid == 0 {
        Some((unsafe { GetCurrentProcess() }, false))
    } else {
        let handle = unsafe { OpenProcess(access, 0, pid) };
        if handle.is_null() {
            None
        } else {
            Some((handle, true))
        }
    }
}

pub fn get_priority(ctx: Ctx<'_>, who: Opt<u32>) -> rquickjs::Result<i32> {
    let pid = who.0.unwrap_or(0);
    let Some((handle, should_close)) = open_priority_handle(pid, PROCESS_QUERY_LIMITED_INFORMATION)
    else {
        return Err(Exception::throw_message(
            &ctx,
            "Failed to get process priority",
        ));
    };
    let class = unsafe { GetPriorityClass(handle) };
    if should_close {
        unsafe { CloseHandle(handle) };
    }
    if class == 0 {
        return Err(Exception::throw_message(
            &ctx,
            "Failed to get process priority",
        ));
    }
    let prio = match class {
        IDLE_PRIORITY_CLASS => 19,
        BELOW_NORMAL_PRIORITY_CLASS => 10,
        NORMAL_PRIORITY_CLASS => 0,
        ABOVE_NORMAL_PRIORITY_CLASS => -7,
        HIGH_PRIORITY_CLASS => -14,
        REALTIME_PRIORITY_CLASS => -20,
        _ => 0,
    };
    Ok(prio)
}

pub fn set_priority(ctx: Ctx<'_>, args: Rest<rquickjs::Value>) -> rquickjs::Result<()> {
    let mut args_iter = args.0.into_iter().rev();
    let prio: i32 = args_iter
        .next()
        .and_then(|v| v.as_number())
        .ok_or_else(|| {
            Exception::throw_type(&ctx, "The `priority` argument must be of type number.")
        })? as i32;
    let pid: u32 = args_iter.next().and_then(|v| v.as_number()).unwrap_or(0f64) as u32;

    if !(-20..=19).contains(&prio) {
        return Err(Exception::throw_range(
            &ctx,
            "The value of `priority` is out of range. It must be >= -20 && <= 19.",
        ));
    }

    let class = if prio < -14 {
        REALTIME_PRIORITY_CLASS
    } else if prio < -7 {
        HIGH_PRIORITY_CLASS
    } else if prio < 0 {
        ABOVE_NORMAL_PRIORITY_CLASS
    } else if prio < 10 {
        NORMAL_PRIORITY_CLASS
    } else if prio < 19 {
        BELOW_NORMAL_PRIORITY_CLASS
    } else {
        IDLE_PRIORITY_CLASS
    };

    let Some((handle, should_close)) = open_priority_handle(pid, PROCESS_SET_INFORMATION) else {
        return Err(Exception::throw_message(
            &ctx,
            "Failed to set process priority",
        ));
    };
    let ok = unsafe { SetPriorityClass(handle, class) };
    if should_close {
        unsafe { CloseHandle(handle) };
    }
    if ok == 0 {
        return Err(Exception::throw_message(
            &ctx,
            "Failed to set process priority",
        ));
    }
    Ok(())
}

pub fn get_type() -> &'static str {
    // In theory there are more types linx MinGW but in practice this is good enough
    "Windows_NT"
}

pub fn get_release() -> &'static str {
    &OS_RELEASE
}

pub fn get_version() -> &'static str {
    &OS_VERSION
}

fn release() -> String {
    let version = OsVersion::current();
    format!("{}.{}.{}", version.major, version.minor, version.build)
}

pub fn get_user_info<'js>(
    ctx: Ctx<'js>,
    _options: Opt<rquickjs::Value>,
) -> rquickjs::Result<Object<'js>> {
    let obj = Object::new(ctx.clone())?;

    obj.set("uid", -1)?;
    obj.set("gid", -1)?;
    obj.set("username", whoami::username().or_throw(&ctx)?)?;
    obj.set("homedir", get_home_dir(ctx.clone()))?;
    obj.set("shell", Null.into_js(&ctx)?)?;
    Ok(obj)
}

fn version() -> Result<String> {
    let version = OsVersion::current();

    let registry_key = LOCAL_MACHINE
        .open("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion")
        .map_err(std::io::Error::from)?;
    let mut value = registry_key
        .get_string("ProductName")
        .map_err(std::io::Error::from)?;
    // Windows 11 shares dwMajorVersion with Windows 10
    // this workaround tries to disambiguate that by checking
    // if the dwBuildNumber is from Windows 11 releases (>= 22000).
    if version.major == 10 && version.build >= 22000 && value.starts_with("Windows 10") {
        value.replace_range(9..10, "1");
    }

    Ok(value)
}
