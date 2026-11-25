// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0

//! Node.js `--permission` model (`--allow-fs-read`, `--allow-fs-write`,
//! `--allow-child-process`, `--allow-worker`, `--allow-addons`, `--allow-wasi`).

use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use rquickjs::{Ctx, Error, Exception};

#[derive(Debug, Clone, Default)]
pub struct PermissionConfig {
    pub fs_read: Vec<String>,
    pub fs_write: Vec<String>,
    pub allow_child: bool,
    pub allow_worker: bool,
    pub allow_addon: bool,
    pub allow_wasi: bool,
}

static ENABLED: AtomicBool = AtomicBool::new(false);
static CONFIG: OnceLock<PermissionConfig> = OnceLock::new();

pub fn init(cfg: PermissionConfig) {
    let _ = CONFIG.set(cfg);
    ENABLED.store(true, Ordering::Release);
}

#[inline]
pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

fn normalize_pattern_or_path(raw: &str) -> String {
    if raw == "*" {
        return "*".to_string();
    }
    let has_trailing_star = raw.ends_with('*');
    let stripped = if has_trailing_star && raw.len() > 1 {
        &raw[..raw.len() - 1]
    } else {
        raw
    };
    let abs = if Path::new(stripped).is_absolute() {
        PathBuf::from(stripped)
    } else {
        std::env::current_dir()
            .unwrap_or_else(|_| PathBuf::from("/"))
            .join(stripped)
    };
    let mut cleaned = PathBuf::new();
    for comp in abs.components() {
        match comp {
            Component::CurDir => {},
            Component::ParentDir => {
                cleaned.pop();
            },
            other => cleaned.push(other.as_os_str()),
        }
    }
    let mut s = cleaned.to_string_lossy().into_owned();
    if has_trailing_star {
        s.push('*');
    }
    s
}

fn matches_fs_list(patterns: &[String], reference: Option<&str>) -> bool {
    let Some(reference) = reference else {
        return patterns.iter().any(|p| p == "*");
    };
    let target = normalize_pattern_or_path(reference);
    for raw_pat in patterns {
        if raw_pat == "*" {
            return true;
        }
        let pat = normalize_pattern_or_path(raw_pat);
        if let Some(prefix) = pat.strip_suffix('*') {
            if target.starts_with(prefix) {
                return true;
            }
            continue;
        }
        if target == pat {
            return true;
        }
        let pat_sep = if pat.ends_with(std::path::MAIN_SEPARATOR) {
            pat.clone()
        } else {
            format!("{pat}{}", std::path::MAIN_SEPARATOR)
        };
        if target.starts_with(&pat_sep) {
            return true;
        }
    }
    false
}

pub fn has(scope: &str, reference: Option<&str>) -> bool {
    if !is_enabled() {
        return true;
    }
    let Some(cfg) = CONFIG.get() else {
        return true;
    };
    match scope {
        "fs" => {
            matches_fs_list(&cfg.fs_read, reference) && matches_fs_list(&cfg.fs_write, reference)
        },
        "fs.read" => matches_fs_list(&cfg.fs_read, reference),
        "fs.write" => matches_fs_list(&cfg.fs_write, reference),
        "child" => cfg.allow_child,
        "worker" => cfg.allow_worker,
        "addon" => cfg.allow_addon,
        "wasi" => cfg.allow_wasi,
        _ => false,
    }
}

pub fn deny_error(ctx: &Ctx<'_>, scope: &str, reference: Option<&str>) -> Error {
    let msg = match reference {
        Some(r) => format!("Access to this API has been restricted ({scope}: {r})"),
        None => format!("Access to this API has been restricted ({scope})"),
    };
    if let Ok(ex) = Exception::from_message(ctx.clone(), &msg) {
        let obj = ex.as_object();
        let _ = obj.set("code", "ERR_ACCESS_DENIED");
        let _ = obj.set("permission", scope);
        if let Some(r) = reference {
            let _ = obj.set("resource", r);
        }
        return ctx.throw(ex.into());
    }
    Exception::throw_message(ctx, &msg)
}

#[inline]
pub fn ensure_fs_read(ctx: &Ctx<'_>, path: &str) -> Result<(), Error> {
    if is_enabled() && !has("fs.read", Some(path)) {
        return Err(deny_error(ctx, "FileSystemRead", Some(path)));
    }
    Ok(())
}

#[inline]
pub fn ensure_fs_write(ctx: &Ctx<'_>, path: &str) -> Result<(), Error> {
    if is_enabled() && !has("fs.write", Some(path)) {
        return Err(deny_error(ctx, "FileSystemWrite", Some(path)));
    }
    Ok(())
}

#[inline]
pub fn ensure_child_process(ctx: &Ctx<'_>) -> Result<(), Error> {
    if is_enabled() && !has("child", None) {
        return Err(deny_error(ctx, "ChildProcess", None));
    }
    Ok(())
}

#[inline]
pub fn ensure_addon(ctx: &Ctx<'_>) -> Result<(), Error> {
    if is_enabled() && !has("addon", None) {
        return Err(deny_error(ctx, "Addon", None));
    }
    Ok(())
}
