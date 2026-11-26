use rquickjs::{Ctx, Exception, Result};
#[cfg(unix)]
use std::os::unix::prelude::PermissionsExt;

use crate::errors::throw_fs_error;

pub(crate) async fn set_mode(ctx: Ctx<'_>, path: &str, mode: u32) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, path)?;
    #[cfg(unix)]
    {
        tokio::fs::set_permissions(path, PermissionsExt::from_mode(mode))
            .await
            .map_err(|err| throw_fs_error(&ctx, err, "chmod", path))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }
    Ok(())
}

pub(crate) fn set_mode_sync(ctx: Ctx<'_>, path: &str, mode: u32) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, path)?;
    #[cfg(unix)]
    {
        std::fs::set_permissions(path, PermissionsExt::from_mode(mode))
            .map_err(|err| throw_fs_error(&ctx, err, "chmod", path))?;
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }
    Ok(())
}

pub async fn chmod(ctx: Ctx<'_>, path: String, mode: u32) -> Result<()> {
    set_mode(ctx, &path, mode).await
}

pub fn chmod_sync(ctx: Ctx<'_>, path: String, mode: u32) -> Result<()> {
    set_mode_sync(ctx, &path, mode)
}

/// `fs.lchmodSync(path, mode)` — change symlink permissions without following.
pub fn lchmod_sync(ctx: Ctx<'_>, path: String, mode: u32) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    #[cfg(unix)]
    {
        let c_path = std::ffi::CString::new(path.as_str())
            .map_err(|_| Exception::throw_type(&ctx, "The \"path\" argument contains NUL bytes"))?;
        // SAFETY: `c_path` is NUL-terminated for the syscall duration.
        let rc = unsafe {
            libc::fchmodat(
                libc::AT_FDCWD,
                c_path.as_ptr(),
                mode as libc::mode_t,
                libc::AT_SYMLINK_NOFOLLOW,
            )
        };
        if rc != 0 {
            return Err(throw_fs_error(
                &ctx,
                std::io::Error::last_os_error(),
                "lchmod",
                &path,
            ));
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
        let _ = &throw_fs_error;
        Err(Exception::throw_message(
            &ctx,
            &format!("ENOSYS: function not implemented, lchmod '{path}'"),
        ))
    }
}

/// `fs.promises.lchmod(path, mode)`.
pub async fn lchmod(ctx: Ctx<'_>, path: String, mode: u32) -> Result<()> {
    lchmod_sync(ctx, path, mode)
}
