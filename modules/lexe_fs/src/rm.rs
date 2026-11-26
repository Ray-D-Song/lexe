// Copyright Amazon.com, Inc. or its affiliates. All Rights Reserved.
// SPDX-License-Identifier: Apache-2.0
use crate::errors::throw_fs_error;
use lexe_utils::result::ResultExt;
use rquickjs::{function::Opt, Ctx, Object, Result};
use tokio::fs;

/// `fs.promises.unlink(path)`.
pub async fn unlink(ctx: Ctx<'_>, path: String) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    fs::remove_file(&path)
        .await
        .map_err(|err| throw_fs_error(&ctx, err, "unlink", &path))?;
    Ok(())
}

/// `fs.unlinkSync(path)`.
pub fn unlink_sync(ctx: Ctx<'_>, path: String) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    std::fs::remove_file(&path).map_err(|err| throw_fs_error(&ctx, err, "unlink", &path))?;
    Ok(())
}

#[allow(clippy::manual_async_fn)]
pub async fn rmdir<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Option<Object<'js>>>,
) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    let recursive = get_params_rm_dir(Opt(options.0.flatten()));

    if recursive {
        fs::remove_dir_all(&path).await
    } else {
        fs::remove_dir(&path).await
    }
    .or_throw_msg(&ctx, &["Can't remove dir \"", &path, "\""].concat())?;

    Ok(())
}

#[allow(clippy::manual_async_fn)]
pub fn rmdir_sync<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Option<Object<'js>>>,
) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    let recursive = get_params_rm_dir(Opt(options.0.flatten()));

    if recursive {
        std::fs::remove_dir_all(&path)
    } else {
        std::fs::remove_dir(&path)
    }
    .or_throw_msg(&ctx, &["Can't remove dir \"", &path, "\""].concat())?;

    Ok(())
}

pub async fn rmfile<'js>(
    ctx: Ctx<'js>,
    path: String,
    options: Opt<Option<Object<'js>>>,
) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    let (recursive, force) = get_params_rm(Opt(options.0.flatten()));

    let res = async move {
        let is_dir = fs::metadata(&path)
            .await
            .map(|metadata| metadata.is_dir())
            .or_throw(&ctx)?;

        (if is_dir && recursive {
            fs::remove_dir_all(&path).await
        } else if is_dir && !recursive {
            fs::remove_dir(&path).await
        } else {
            fs::remove_file(&path).await
        })
        .or_throw_msg(&ctx, &["Can't remove file \"", &path, "\""].concat())?;

        Ok(())
    }
    .await;

    if !force {
        return res;
    }

    Ok(())
}

pub fn rmfile_sync(ctx: Ctx<'_>, path: String, options: Opt<Option<Object<'_>>>) -> Result<()> {
    lexe_utils::permissions::ensure_fs_write(&ctx, &path)?;
    let (recursive, force) = get_params_rm(Opt(options.0.flatten()));

    let res = (|| -> Result<()> {
        let is_dir = std::fs::metadata(&path).map(|metadata| metadata.is_dir())?;

        (if is_dir && recursive {
            std::fs::remove_dir_all(&path)
        } else if is_dir && !recursive {
            std::fs::remove_dir(&path)
        } else {
            std::fs::remove_file(&path)
        })?;

        Ok(())
    })();

    if !force {
        return res;
    }

    Ok(())
}

fn get_params_rm_dir(options: Opt<Object>) -> bool {
    let mut recursive = false;

    if let Some(options) = options.0 {
        recursive = options.get("recursive").unwrap_or_default();
    }
    recursive
}

fn get_params_rm(options: Opt<Object>) -> (bool, bool) {
    let mut recursive = false;
    let mut force = false;

    if let Some(options) = options.0 {
        recursive = options.get("recursive").unwrap_or_default();
        force = options.get("force").unwrap_or_default();
    }
    (recursive, force)
}
