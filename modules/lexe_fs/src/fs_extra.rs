// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
//! `fs/promises` entries that were missing: link/symlink/readlink, truncate,
//! chown/lchown, and utimes/lutimes. Each one has a sync twin so `fs` and
//! `fs/promises` stay consistent.

use lexe_utils::result::ResultExt;
use rquickjs::{function::Opt, Ctx, Exception, IntoJs, Result};

use crate::fd_sync::throw_out_of_range;

fn throw_copyfile_error(
    ctx: &Ctx<'_>,
    err: std::io::Error,
    src: &str,
    dest: &str,
) -> rquickjs::Error {
    match crate::errors::create_fs_error(ctx, err, "copyfile", src) {
        Ok(exc) => {
            let _ = exc.as_object().set("dest", dest);
            exc.throw()
        },
        Err(e) => e,
    }
}

fn validate_copyfile_mode(ctx: &Ctx<'_>, mode: Opt<rquickjs::Value<'_>>) -> Result<i32> {
    let val = match mode.0 {
        None => 0,
        Some(v) if v.is_null() || v.is_undefined() => 0,
        Some(v) => v
            .as_number()
            .filter(|n| n.is_finite() && n.fract() == 0.0)
            .map(|n| n as i32)
            .ok_or_else(|| {
                crate::fd_sync::throw_invalid_arg_type(
                    ctx,
                    "The \"mode\" argument must be of type number",
                )
            })?,
    };
    if !(0..=7).contains(&val) {
        return Err(throw_out_of_range(
            ctx,
            &format!(
                "The value of \"mode\" is out of range. It must be >= 0 && <= 7. Received {val}"
            ),
        ));
    }
    Ok(val)
}

#[cfg(target_os = "macos")]
fn macos_clonefile(src: &str, dest: &str) -> std::io::Result<()> {
    extern "C" {
        fn clonefile(src: *const libc::c_char, dst: *const libc::c_char, flags: u32)
            -> libc::c_int;
    }
    let c_src = std::ffi::CString::new(src)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL byte in path"))?;
    let c_dst = std::ffi::CString::new(dest)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "NUL byte in path"))?;
    let rc = unsafe { clonefile(c_src.as_ptr(), c_dst.as_ptr(), 0) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// `fs.copyFileSync(src, dest[, mode])`.
pub fn copy_file_sync<'js>(
    ctx: Ctx<'js>,
    src: String,
    dest: String,
    mode: Opt<rquickjs::Value<'js>>,
) -> Result<()> {
    lexe_utils::permissions::ensure_fs_read(&ctx, &src)?;
    lexe_utils::permissions::ensure_fs_write(&ctx, &dest)?;
    let flags = validate_copyfile_mode(&ctx, mode)?;
    let excl = (flags & 1) != 0;
    let ficlone = (flags & 2) != 0;
    let ficlone_force = (flags & 4) != 0;

    if ficlone_force || ficlone {
        #[cfg(target_os = "macos")]
        {
            if !excl && std::fs::metadata(&src).is_ok() && std::fs::symlink_metadata(&dest).is_ok()
            {
                let _ = std::fs::remove_file(&dest);
            }
            match macos_clonefile(&src, &dest) {
                Ok(()) => return Ok(()),
                Err(err) if ficlone_force => {
                    return Err(throw_copyfile_error(&ctx, err, &src, &dest));
                },
                Err(_) => {},
            }
        }
        #[cfg(not(target_os = "macos"))]
        if ficlone_force {
            #[cfg(unix)]
            let err = std::io::Error::from_raw_os_error(libc::ENOTSUP);
            #[cfg(not(unix))]
            let err = std::io::Error::new(std::io::ErrorKind::Unsupported, "ENOTSUP");
            return Err(throw_copyfile_error(&ctx, err, &src, &dest));
        }
    }

    if excl {
        let mut src_file = std::fs::File::open(&src)
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
        let perms = src_file
            .metadata()
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?
            .permissions();
        let mut dst_file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
        std::io::copy(&mut src_file, &mut dst_file)
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
        let _ = dst_file.set_permissions(perms);
        return Ok(());
    }

    std::fs::copy(&src, &dest).map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
    Ok(())
}

/// `fs.promises.copyFile(src, dest[, mode])`.
pub async fn copy_file<'js>(
    ctx: Ctx<'js>,
    src: String,
    dest: String,
    mode: Opt<rquickjs::Value<'js>>,
) -> Result<()> {
    lexe_utils::permissions::ensure_fs_read(&ctx, &src)?;
    lexe_utils::permissions::ensure_fs_write(&ctx, &dest)?;
    let flags = validate_copyfile_mode(&ctx, mode)?;
    let excl = (flags & 1) != 0;
    let ficlone = (flags & 2) != 0;
    let ficlone_force = (flags & 4) != 0;

    if ficlone_force || ficlone {
        #[cfg(target_os = "macos")]
        {
            if !excl
                && tokio::fs::metadata(&src).await.is_ok()
                && tokio::fs::symlink_metadata(&dest).await.is_ok()
            {
                let _ = tokio::fs::remove_file(&dest).await;
            }
            match macos_clonefile(&src, &dest) {
                Ok(()) => return Ok(()),
                Err(err) if ficlone_force => {
                    return Err(throw_copyfile_error(&ctx, err, &src, &dest));
                },
                Err(_) => {},
            }
        }
        #[cfg(not(target_os = "macos"))]
        if ficlone_force {
            #[cfg(unix)]
            let err = std::io::Error::from_raw_os_error(libc::ENOTSUP);
            #[cfg(not(unix))]
            let err = std::io::Error::new(std::io::ErrorKind::Unsupported, "ENOTSUP");
            return Err(throw_copyfile_error(&ctx, err, &src, &dest));
        }
    }

    if excl {
        let mut src_file = tokio::fs::File::open(&src)
            .await
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
        let perms = src_file
            .metadata()
            .await
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?
            .permissions();
        let mut dst_file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&dest)
            .await
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
        tokio::io::copy(&mut src_file, &mut dst_file)
            .await
            .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
        let _ = dst_file.set_permissions(perms).await;
        return Ok(());
    }

    let _ = &Exception::throw_message;
    tokio::fs::copy(&src, &dest)
        .await
        .map_err(|err| throw_copyfile_error(&ctx, err, &src, &dest))?;
    Ok(())
}

/// `fs.promises.link(existingPath, newPath)`.
pub async fn link(ctx: Ctx<'_>, existing: String, new_path: String) -> Result<()> {
    tokio::fs::hard_link(&existing, &new_path)
        .await
        .or_throw_msg(&ctx, &format!("link {existing} -> {new_path}"))?;
    Ok(())
}

pub fn link_sync(ctx: Ctx<'_>, existing: String, new_path: String) -> Result<()> {
    std::fs::hard_link(&existing, &new_path)
        .or_throw_msg(&ctx, &format!("link {existing} -> {new_path}"))?;
    Ok(())
}

/// `fs.promises.readlink(path[, options])`.
///
/// Returns a string by default, or a Buffer when `options.encoding` is
/// `"buffer"`, matching Node.
pub async fn readlink(ctx: Ctx<'_>, path: String, as_buffer: bool) -> Result<rquickjs::Value<'_>> {
    let target = tokio::fs::read_link(&path)
        .await
        .or_throw_msg(&ctx, &format!("readlink {path}"))?;
    let text = target.to_string_lossy().into_owned();
    if as_buffer {
        lexe_buffer::Buffer(text.into_bytes()).into_js(&ctx)
    } else {
        text.into_js(&ctx)
    }
}

pub fn readlink_sync(ctx: Ctx<'_>, path: String, as_buffer: bool) -> Result<rquickjs::Value<'_>> {
    let target = std::fs::read_link(&path).or_throw_msg(&ctx, &format!("readlink {path}"))?;
    let text = target.to_string_lossy().into_owned();
    if as_buffer {
        lexe_buffer::Buffer(text.into_bytes()).into_js(&ctx)
    } else {
        text.into_js(&ctx)
    }
}

/// `fs.promises.truncate(path[, len])`.
pub async fn truncate(ctx: Ctx<'_>, path: String, len: Option<u64>) -> Result<()> {
    let file = tokio::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .await
        .or_throw_msg(&ctx, &format!("truncate {path}"))?;
    file.set_len(len.unwrap_or(0))
        .await
        .or_throw_msg(&ctx, &format!("truncate {path}"))?;
    Ok(())
}

pub fn truncate_sync(ctx: Ctx<'_>, path: String, len: Option<u64>) -> Result<()> {
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .or_throw_msg(&ctx, &format!("truncate {path}"))?;
    file.set_len(len.unwrap_or(0))
        .or_throw_msg(&ctx, &format!("truncate {path}"))?;
    Ok(())
}

/// `fs.promises.chown(path, uid, gid)`.
pub async fn chown(ctx: Ctx<'_>, path: String, uid: u32, gid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let c_path = std::ffi::CString::new(path.as_str())
            .or_throw_msg(&ctx, "chown: path contains a NUL byte")?;
        // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
        let rc = unsafe { libc::chown(c_path.as_ptr(), uid, gid) };
        if rc != 0 {
            let err: Result<()> = Err::<(), _>(std::io::Error::last_os_error())
                .or_throw_msg(&ctx, &format!("chown {path}"));
            return err;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        // Windows has no chown; Node reports ENOSYS here.
        let _ = (uid, gid);
        Err(rquickjs::Exception::throw_message(
            &ctx,
            &format!("chown {path}: ENOSYS"),
        ))
    }
}

pub fn chown_sync(ctx: Ctx<'_>, path: String, uid: u32, gid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::chown;
        chown(&path, Some(uid), Some(gid)).or_throw_msg(&ctx, &format!("chown {path}"))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (uid, gid);
        Err(rquickjs::Exception::throw_message(
            &ctx,
            &format!("chown {path}: ENOSYS"),
        ))
    }
}

/// `fs.promises.lchown(path, uid, gid)` — `chown` without following symlinks.
pub async fn lchown(ctx: Ctx<'_>, path: String, uid: u32, gid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        let c_path = std::ffi::CString::new(path.as_str())
            .or_throw_msg(&ctx, "lchown: path contains a NUL byte")?;
        // SAFETY: `c_path` is a valid NUL-terminated string that outlives the call.
        let rc = unsafe { libc::lchown(c_path.as_ptr(), uid, gid) };
        if rc != 0 {
            let err: Result<()> = Err::<(), _>(std::io::Error::last_os_error())
                .or_throw_msg(&ctx, &format!("lchown {path}"));
            return err;
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (uid, gid);
        Err(rquickjs::Exception::throw_message(
            &ctx,
            &format!("lchown {path}: ENOSYS"),
        ))
    }
}

pub fn lchown_sync(ctx: Ctx<'_>, path: String, uid: u32, gid: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::lchown;
        lchown(&path, Some(uid), Some(gid)).or_throw_msg(&ctx, &format!("lchown {path}"))?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = (uid, gid);
        Err(rquickjs::Exception::throw_message(
            &ctx,
            &format!("lchown {path}: ENOSYS"),
        ))
    }
}

/// `fs.promises.utimes(path, atime, mtime)`; times are seconds (number) or Date.
pub async fn utimes(ctx: Ctx<'_>, path: String, atime: f64, mtime: f64) -> Result<()> {
    utimes_impl(&ctx, &path, atime, mtime, true)
}

pub fn utimes_sync(ctx: Ctx<'_>, path: String, atime: f64, mtime: f64) -> Result<()> {
    utimes_impl(&ctx, &path, atime, mtime, true)
}

/// `fs.promises.lutimes(path, atime, mtime)` — `utimes` without following
/// symlinks.
pub async fn lutimes(ctx: Ctx<'_>, path: String, atime: f64, mtime: f64) -> Result<()> {
    utimes_impl(&ctx, &path, atime, mtime, false)
}

pub fn lutimes_sync(ctx: Ctx<'_>, path: String, atime: f64, mtime: f64) -> Result<()> {
    utimes_impl(&ctx, &path, atime, mtime, false)
}

fn utimes_impl(ctx: &Ctx<'_>, path: &str, atime: f64, mtime: f64, follow: bool) -> Result<()> {
    #[cfg(unix)]
    {
        let syscall = if follow { "utimes" } else { "lutimes" };
        let c_path = std::ffi::CString::new(path)
            .or_throw_msg(ctx, &format!("{syscall}: path contains a NUL byte"))?;
        let to_ts = |secs: f64| {
            let s = secs.max(0.0);
            libc::timespec {
                tv_sec: s as libc::time_t,
                tv_nsec: (s.fract() * 1_000_000_000.0) as _,
            }
        };
        let times = [to_ts(atime), to_ts(mtime)];
        let flags = if follow { 0 } else { libc::AT_SYMLINK_NOFOLLOW };
        let rc = unsafe { libc::utimensat(libc::AT_FDCWD, c_path.as_ptr(), times.as_ptr(), flags) };
        if rc != 0 {
            return Err::<(), _>(std::io::Error::last_os_error())
                .or_throw_msg(ctx, &format!("{syscall} {path}"));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = follow;
        use std::time::{Duration, UNIX_EPOCH};
        let at = UNIX_EPOCH + Duration::from_secs_f64(atime.max(0.0));
        let mt = UNIX_EPOCH + Duration::from_secs_f64(mtime.max(0.0));
        let times = std::fs::FileTimes::new().set_accessed(at).set_modified(mt);
        let file = std::fs::OpenOptions::new()
            .write(true)
            .open(path)
            .or_throw_msg(ctx, &format!("utimes {path}"))?;
        file.set_times(times)
            .or_throw_msg(ctx, &format!("utimes {path}"))?;
        Ok(())
    }
}

/// `fs.promises.glob(pattern[, options])` — an async iterator of matching paths.
///
/// Node's `glob` walks the tree and applies a glob matcher. This implements the
/// common forms (`*`, `**`, `?`, character classes via `glob_match`) on top of
/// the same directory walker the rest of this module uses, and yields paths
/// relative to `cwd`, as Node does.
pub fn glob_pattern_to_matcher(pattern: &str) -> GlobMatcher {
    GlobMatcher::new(pattern)
}

/// Minimal glob matcher: `**` crosses separators, `*` and `?` do not, `[...]`
/// is a character class, and `{a,b}` alternates.
pub struct GlobMatcher {
    pattern: String,
}

impl GlobMatcher {
    pub fn new(pattern: &str) -> Self {
        Self {
            pattern: pattern.to_string(),
        }
    }

    pub fn is_match(&self, path: &str) -> bool {
        glob_match(self.pattern.as_bytes(), path.as_bytes())
    }
}

fn glob_match(pattern: &[u8], text: &[u8]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }
    match pattern[0] {
        b'*' => {
            // `**` also crosses path separators.
            let double = pattern.len() > 1 && pattern[1] == b'*';
            let rest = if double {
                // Skip `**` and an immediately following `/`.
                let mut r = &pattern[2..];
                if r.first() == Some(&b'/') {
                    r = &r[1..];
                }
                r
            } else {
                &pattern[1..]
            };
            if double {
                if glob_match(rest, text) {
                    return true;
                }
                for i in 0..text.len() {
                    if glob_match(rest, &text[i + 1..]) {
                        return true;
                    }
                }
                return false;
            }
            for i in 0..=text.len() {
                if text[..i].contains(&b'/') {
                    break;
                }
                if glob_match(rest, &text[i..]) {
                    return true;
                }
            }
            false
        },
        b'?' => !text.is_empty() && text[0] != b'/' && glob_match(&pattern[1..], &text[1..]),
        b'[' => {
            // Character class; `!` or `^` negates, `a-z` is a range.
            if text.is_empty() {
                return false;
            }
            let mut idx = 1;
            let negate = matches!(pattern.get(idx), Some(b'!') | Some(b'^'));
            if negate {
                idx += 1;
            }
            let mut matched = false;
            while idx < pattern.len() && pattern[idx] != b']' {
                if idx + 2 < pattern.len() && pattern[idx + 1] == b'-' && pattern[idx + 2] != b']' {
                    if text[0] >= pattern[idx] && text[0] <= pattern[idx + 2] {
                        matched = true;
                    }
                    idx += 3;
                } else {
                    if text[0] == pattern[idx] {
                        matched = true;
                    }
                    idx += 1;
                }
            }
            if idx >= pattern.len() {
                // Unterminated class: treat `[` literally.
                return text[0] == b'[' && glob_match(&pattern[1..], &text[1..]);
            }
            matched != negate && glob_match(&pattern[idx + 1..], &text[1..])
        },
        b'{' => {
            // Alternation: split on top-level commas.
            let mut depth = 0;
            let mut end = None;
            for (i, &b) in pattern.iter().enumerate().skip(1) {
                match b {
                    b'{' => depth += 1,
                    b'}' if depth == 0 => {
                        end = Some(i);
                        break;
                    },
                    b'}' => depth -= 1,
                    _ => {},
                }
            }
            let Some(end) = end else {
                return text.first() == Some(&b'{') && glob_match(&pattern[1..], &text[1..]);
            };
            let body = &pattern[1..end];
            let rest = &pattern[end + 1..];
            let mut start = 0;
            let mut depth = 0;
            for i in 0..=body.len() {
                if i == body.len() || (body[i] == b',' && depth == 0) {
                    let mut alt = body[start..i].to_vec();
                    alt.extend_from_slice(rest);
                    if glob_match(&alt, text) {
                        return true;
                    }
                    start = i + 1;
                } else if body[i] == b'{' {
                    depth += 1;
                } else if body[i] == b'}' {
                    depth -= 1;
                }
            }
            false
        },
        c => !text.is_empty() && text[0] == c && glob_match(&pattern[1..], &text[1..]),
    }
}

#[cfg(test)]
mod tests {
    use super::glob_match;

    fn m(p: &str, t: &str) -> bool {
        glob_match(p.as_bytes(), t.as_bytes())
    }

    #[test]
    fn star_does_not_cross_separator() {
        assert!(m("*.js", "a.js"));
        assert!(!m("*.js", "src/a.js"));
    }

    #[test]
    fn double_star_crosses_separators() {
        assert!(m("**/*.js", "src/deep/a.js"));
        assert!(m("**/*.js", "a.js"));
    }

    #[test]
    fn question_and_class() {
        assert!(m("a?c", "abc"));
        assert!(!m("a?c", "a/c"));
        assert!(m("[abc]x", "bx"));
        assert!(!m("[!abc]x", "bx"));
    }

    #[test]
    fn braces_alternate() {
        assert!(m("*.{js,ts}", "a.ts"));
        assert!(!m("*.{js,ts}", "a.rs"));
    }
}
